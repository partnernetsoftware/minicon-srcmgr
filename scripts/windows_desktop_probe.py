"""Opt-in DXGI desktop pixel/QPC observer, independent of product surfaces.

Uses DXGI_OUTDUPL_FRAME_INFO.LastPresentTime paired with the actual acquired
desktop pixel. Readback completion is recorded separately, never subtracted.
No screenshots or desktop contents are persisted by this helper.
"""
import ctypes as c
from ctypes import wintypes as w
import uuid


class Guid(c.Structure):
    _fields_ = [("data", c.c_ubyte * 16)]
    def __init__(self, text):
        super().__init__((c.c_ubyte * 16).from_buffer_copy(uuid.UUID(text).bytes_le))


def call(obj, index, result, args, *values):
    table = c.cast(obj, c.POINTER(c.POINTER(c.c_void_p))).contents
    return c.WINFUNCTYPE(result, c.c_void_p, *args)(table[index])(obj, *values)


def release(obj):
    if obj:
        call(obj, 2, w.ULONG, [])


def checked(result, operation):
    if result < 0:
        raise RuntimeError(f"{operation}: HRESULT 0x{result & 0xffffffff:08x}")


class Output(c.Structure):
    _fields_ = [("name", w.WCHAR * 32), ("rect", w.RECT), ("attached", w.BOOL), ("rotation", w.UINT), ("monitor", w.HANDLE)]


class Pointer(c.Structure):
    _fields_ = [("position", w.POINT), ("visible", w.BOOL)]


class Frame(c.Structure):
    _fields_ = [("present", c.c_longlong), ("mouse", c.c_longlong), ("accumulated", w.UINT), ("coalesced", w.BOOL), ("protected", w.BOOL), ("pointer", Pointer), ("metadata", w.UINT), ("shape", w.UINT)]


class Texture(c.Structure):
    _fields_ = [(name, w.UINT) for name in ("width", "height", "mips", "array", "format", "samples", "quality", "usage", "bind", "cpu", "misc")]


class Mapped(c.Structure):
    _fields_ = [("data", c.c_void_p), ("row_pitch", w.UINT), ("depth_pitch", w.UINT)]


class Box(c.Structure):
    _fields_ = [(name, w.UINT) for name in ("left", "top", "front", "right", "bottom", "back")]


class DesktopSampler:
    def __init__(self, x, y):
        self.owned = []
        self.frame_held = False
        self.frequency = c.c_longlong()
        kernel = c.WinDLL("kernel32")
        kernel.QueryPerformanceFrequency(c.byref(self.frequency))
        try:
            dxgi = c.WinDLL("dxgi")
            dxgi.CreateDXGIFactory1.argtypes = [c.c_void_p, c.POINTER(c.c_void_p)]
            dxgi.CreateDXGIFactory1.restype = c.c_long
            factory = c.c_void_p()
            checked(dxgi.CreateDXGIFactory1(c.byref(Guid("770aae78-f26f-4dba-a829-253c83d1b387")), c.byref(factory)), "DXGI factory")
            self.owned.append(factory)
            selected = None
            for ai in range(8):
                adapter = c.c_void_p()
                if call(factory, 12, c.c_long, [w.UINT, c.POINTER(c.c_void_p)], ai, c.byref(adapter)) < 0:
                    break
                self.owned.append(adapter)
                for oi in range(8):
                    output = c.c_void_p()
                    if call(adapter, 7, c.c_long, [w.UINT, c.POINTER(c.c_void_p)], oi, c.byref(output)) < 0:
                        break
                    self.owned.append(output)
                    description = Output()
                    checked(call(output, 7, c.c_long, [c.POINTER(Output)], c.byref(description)), "output description")
                    rect = description.rect
                    if description.attached and rect.left <= x < rect.right and rect.top <= y < rect.bottom:
                        if description.rotation not in (0, 1):
                            raise RuntimeError("rotated desktop observation is not qualified")
                        selected = adapter, output, rect
                        break
                if selected:
                    break
            if not selected:
                raise RuntimeError("no desktop output contains the observed point")
            adapter, output, self.rect = selected
            output1 = c.c_void_p()
            checked(call(output, 0, c.c_long, [c.c_void_p, c.POINTER(c.c_void_p)], c.byref(Guid("00cddea8-939b-4b83-a340-a685226666cc")), c.byref(output1)), "DXGI output1")
            self.owned.append(output1)
            d3d = c.WinDLL("d3d11")
            d3d.D3D11CreateDevice.argtypes = [c.c_void_p, w.UINT, w.HMODULE, w.UINT, c.c_void_p, w.UINT, w.UINT, c.POINTER(c.c_void_p), c.c_void_p, c.POINTER(c.c_void_p)]
            d3d.D3D11CreateDevice.restype = c.c_long
            self.device, self.context = c.c_void_p(), c.c_void_p()
            checked(d3d.D3D11CreateDevice(adapter, 0, None, 0, None, 0, 7, c.byref(self.device), None, c.byref(self.context)), "D3D device")
            self.owned.extend([self.device, self.context])
            self.dup = c.c_void_p()
            checked(call(output1, 22, c.c_long, [c.c_void_p, c.POINTER(c.c_void_p)], self.device, c.byref(self.dup)), "duplicate desktop")
            self.owned.append(self.dup)
            self.staging = c.c_void_p()
            description = Texture(1, 1, 1, 1, 87, 1, 0, 3, 0, 0x20000, 0)
            checked(call(self.device, 5, c.c_long, [c.POINTER(Texture), c.c_void_p, c.POINTER(c.c_void_p)], c.byref(description), None, c.byref(self.staging)), "staging pixel")
            self.owned.append(self.staging)
        except BaseException:
            self.close()
            raise

    def read(self, x, y, timeout_ms=20):
        if not (self.rect.left <= x < self.rect.right and self.rect.top <= y < self.rect.bottom):
            raise RuntimeError("observed point moved outside the selected desktop")
        frame, resource = Frame(), c.c_void_p()
        result = call(self.dup, 8, c.c_long, [w.UINT, c.POINTER(Frame), c.POINTER(c.c_void_p)], timeout_ms, c.byref(frame), c.byref(resource))
        if result & 0xffffffff == 0x887a0027:  # DXGI_ERROR_WAIT_TIMEOUT
            return None
        checked(result, "acquire desktop frame")
        self.frame_held = True
        texture = c.c_void_p()
        try:
            # Initial empty and pointer-only acquisitions do not timestamp a
            # desktop image update and cannot establish visible feedback.
            if frame.present == 0:
                return None
            checked(call(resource, 0, c.c_long, [c.c_void_p, c.POINTER(c.c_void_p)], c.byref(Guid("6f15aaf2-d208-4e89-9ab4-489535d34f9c")), c.byref(texture)), "desktop texture")
            px, py = x - self.rect.left, y - self.rect.top
            box = Box(px, py, 0, px + 1, py + 1, 1)
            call(self.context, 46, None, [c.c_void_p, w.UINT, w.UINT, w.UINT, w.UINT, c.c_void_p, w.UINT, c.POINTER(Box)], self.staging, 0, 0, 0, 0, texture, 0, c.byref(box))
            mapped = Mapped()
            checked(call(self.context, 14, c.c_long, [c.c_void_p, w.UINT, w.UINT, w.UINT, c.POINTER(Mapped)], self.staging, 0, 1, 0, c.byref(mapped)), "map desktop pixel")
            try:
                value = c.cast(mapped.data, c.POINTER(c.c_uint32))[0]
            finally:
                call(self.context, 15, None, [c.c_void_p, w.UINT], self.staging, 0)
            pixel = ((value & 255) << 16) | (value & 0xff00) | ((value >> 16) & 255)
            return {"pixel": pixel, "present_qpc": frame.present, "present_seconds": frame.present / self.frequency.value,
                    "accumulated": frame.accumulated, "protected": bool(frame.protected)}
        finally:
            release(texture)
            release(resource)
            checked(call(self.dup, 14, c.c_long, []), "release desktop frame")
            self.frame_held = False

    def close(self):
        for obj in reversed(self.owned):
            release(obj)
        self.owned.clear()
