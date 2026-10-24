//! Windows-only COM ownership and HWND subclass. No VCS or profile access.
#![allow(non_snake_case, non_upper_case_globals)]
use crate::{Action, Error, MAX_TEXT, PublishedNode, PublishedRole, Shared, Snapshot};
use std::{
    ffi::c_void,
    marker::PhantomData,
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
use windows::Win32::UI::Input::KeyboardAndMouse::SetFocus;
use windows::core as wcore;
use windows::{
    Win32::{
        Foundation::*,
        System::{Com::*, Ole::*, Threading::*, Variant::*},
        UI::{Accessibility::*, Shell::*, WindowsAndMessaging::*},
    },
    core::{
        BOOL, BSTR, HRESULT, IUnknown, IUnknown_Vtbl, IUnknownImpl, Interface, PCWSTR, implement,
        interface,
    },
};

// Raw nullable outputs retain the official ABI while always initializing null
// at end-of-navigation/unsupported-pattern. Generated COM ownership is reused.
#[interface("d6dd68d1-86fd-4332-8666-9abedea2d24c")]
unsafe trait Simple: IUnknown {
    fn ProviderOptions(&self, out: *mut ProviderOptions) -> HRESULT;
    fn GetPatternProvider(&self, id: UIA_PATTERN_ID, out: *mut *mut c_void) -> HRESULT;
    fn GetPropertyValue(&self, id: UIA_PROPERTY_ID, out: *mut VARIANT) -> HRESULT;
    fn HostRawElementProvider(&self, out: *mut *mut c_void) -> HRESULT;
}
#[interface("f7063da8-8359-439c-9297-bbc5299a7d87")]
unsafe trait Fragment: IUnknown {
    fn Navigate(&self, direction: NavigateDirection, out: *mut *mut c_void) -> HRESULT;
    fn GetRuntimeId(&self, out: *mut *mut SAFEARRAY) -> HRESULT;
    fn BoundingRectangle(&self, out: *mut UiaRect) -> HRESULT;
    fn GetEmbeddedFragmentRoots(&self, out: *mut *mut SAFEARRAY) -> HRESULT;
    fn SetFocus(&self) -> HRESULT;
    fn FragmentRoot(&self, out: *mut *mut c_void) -> HRESULT;
}
#[interface("620ce2a5-ab8f-40a9-86cb-de3c75599b58")]
unsafe trait FragmentRoot: IUnknown {
    fn ElementProviderFromPoint(&self, x: f64, y: f64, out: *mut *mut c_void) -> HRESULT;
    fn GetFocus(&self, out: *mut *mut c_void) -> HRESULT;
}
fn unavailable() -> wcore::Error {
    wcore::Error::from_hresult(HRESULT(UIA_E_ELEMENTNOTAVAILABLE as i32))
}
unsafe fn output<T: Default>(out: *mut T, result: wcore::Result<T>) -> HRESULT {
    if out.is_null() {
        return E_POINTER;
    }
    // SAFETY: COM supplies a valid output slot; initialize even on failure.
    unsafe {
        out.write(T::default());
    }
    match result {
        Ok(value) => {
            unsafe {
                out.write(value);
            }
            HRESULT(0)
        }
        Err(error) => error.code(),
    }
}
#[derive(Clone)]
struct Element {
    shared: Arc<Shared>,
    hwnd: isize,
    id: u32,
    context: Option<u64>,
    name: Option<String>,
}
impl Element {
    fn current(&self) -> wcore::Result<Arc<Snapshot>> {
        let guard = self.shared.snapshot.lock().map_err(|_| unavailable())?;
        let snapshot = guard.as_ref().ok_or_else(unavailable)?;
        if self
            .context
            .is_some_and(|context| context != snapshot.context)
            || snapshot.tree.node(self.id).is_none()
            || self.name.as_ref().is_some_and(|name| {
                snapshot
                    .tree
                    .node(self.id)
                    .is_none_or(|node| &node.name != name)
            })
        {
            return Err(unavailable());
        }
        Ok(snapshot.clone())
    }
    fn node<'a>(&self, snapshot: &'a Snapshot) -> wcore::Result<&'a PublishedNode> {
        snapshot.tree.node(self.id).ok_or_else(unavailable)
    }
    fn fragment(&self, id: u32, context: u64) -> wcore::Result<IRawElementProviderFragment> {
        let snapshot = self.current()?;
        if snapshot.context != context {
            return Err(unavailable());
        }
        let mut element = self.clone();
        element.id = id;
        element.context = if id == 1 { None } else { Some(context) };
        element.name = if id == 1 {
            None
        } else {
            Some(snapshot.tree.node(id).ok_or_else(unavailable)?.name.clone())
        };
        let simple: Simple = if id == 1 {
            Root { element }.into()
        } else {
            Child { element }.into()
        };
        simple.cast()
    }
    fn navigate(&self, direction: NavigateDirection) -> wcore::Result<*mut c_void> {
        let snapshot = self.current()?;
        let node = self.node(&snapshot)?;
        let siblings: Vec<_> = snapshot
            .tree
            .nodes
            .iter()
            .filter(|n| n.parent == node.parent)
            .map(|n| n.id)
            .collect();
        let position = siblings
            .iter()
            .position(|id| *id == self.id)
            .ok_or_else(unavailable)?;
        let children = snapshot.tree.children_of(self.id);
        let target = match direction {
            NavigateDirection_Parent => node.parent,
            NavigateDirection_FirstChild => children.first().copied(),
            NavigateDirection_LastChild => children.last().copied(),
            NavigateDirection_NextSibling => siblings.get(position + 1).copied(),
            NavigateDirection_PreviousSibling => position
                .checked_sub(1)
                .and_then(|p| siblings.get(p).copied()),
            _ => None,
        };
        target
            .map(|id| self.fragment(id, snapshot.context).map(Interface::into_raw))
            .transpose()
            .map(|v| v.unwrap_or(std::ptr::null_mut()))
    }
    fn bounds(&self) -> wcore::Result<UiaRect> {
        let snapshot = self.current()?;
        let bounds = &self.node(&snapshot)?.bounds;
        let mut point = POINT {
            x: bounds.x,
            y: bounds.y,
        };
        // SAFETY: owned live HWND; POINT is stack-owned for this call.
        unsafe {
            windows::Win32::Graphics::Gdi::ClientToScreen(
                HWND(self.hwnd as *mut c_void),
                &mut point,
            )
            .ok()?;
        }
        Ok(UiaRect {
            left: point.x as f64,
            top: point.y as f64,
            width: bounds.width as f64,
            height: bounds.height as f64,
        })
    }
    fn property(&self, id: UIA_PROPERTY_ID) -> wcore::Result<VARIANT> {
        let snapshot = self.current()?;
        let node = self.node(&snapshot)?;
        Ok(match id {
            UIA_NamePropertyId => VARIANT::from(node.name.as_str()),
            UIA_ProcessIdPropertyId => VARIANT::from(unsafe { GetCurrentProcessId() } as i32),
            UIA_NativeWindowHandlePropertyId if self.id == 1 => VARIANT::from(self.hwnd as i32),
            UIA_ProviderDescriptionPropertyId => VARIANT::from("MiniCon SrcMgr UIA provider"),
            UIA_AutomationIdPropertyId => {
                VARIANT::from(format!("srcmgr-{}-{}", snapshot.context, node.id).as_str())
            }
            UIA_FrameworkIdPropertyId => VARIANT::from("MiniCon SrcMgr"),
            UIA_ControlTypePropertyId => VARIANT::from(
                match node.role {
                    PublishedRole::Button => UIA_ButtonControlTypeId,
                    PublishedRole::Text if node.editable => UIA_EditControlTypeId,
                    PublishedRole::Text | PublishedRole::Terminal => UIA_DocumentControlTypeId,
                    PublishedRole::Label => UIA_TextControlTypeId,
                    PublishedRole::Frame => UIA_WindowControlTypeId,
                    _ => UIA_PaneControlTypeId,
                }
                .0,
            ),
            UIA_IsEnabledPropertyId => VARIANT::from(
                !snapshot.disabled.contains(&self.id)
                    && self.shared.accepting.load(Ordering::Acquire),
            ),
            UIA_IsKeyboardFocusablePropertyId => VARIANT::from(node.focusable),
            UIA_HasKeyboardFocusPropertyId => VARIANT::from(node.focused),
            UIA_IsControlElementPropertyId | UIA_IsContentElementPropertyId => VARIANT::from(true),
            UIA_IsOffscreenPropertyId => {
                VARIANT::from(node.bounds.width == 0 || node.bounds.height == 0)
            }
            UIA_ValueValuePropertyId => VARIANT::from(node.text.as_str()),
            UIA_ValueIsReadOnlyPropertyId => VARIANT::from(!node.editable),
            _ => VARIANT::default(),
        })
    }
    fn request(&self, action: Action) -> wcore::Result<()> {
        let snapshot = self.current()?;
        if self.shared.request(snapshot.revision, self.id, action) {
            Ok(())
        } else {
            Err(wcore::Error::from_hresult(HRESULT(
                UIA_E_ELEMENTNOTENABLED as i32,
            )))
        }
    }
    fn root(&self) -> wcore::Result<*mut c_void> {
        let snapshot = self.current()?;
        self.fragment(1, snapshot.context)?
            .cast::<IRawElementProviderFragmentRoot>()
            .map(Interface::into_raw)
    }
    fn pattern(&self, id: UIA_PATTERN_ID, unknown: IUnknown) -> wcore::Result<*mut c_void> {
        let snapshot = self.current()?;
        let node = self.node(&snapshot)?;
        if id == UIA_InvokePatternId && node.clickable {
            unknown.cast::<IInvokeProvider>().map(Interface::into_raw)
        } else if id == UIA_ValuePatternId && (node.editable || !node.text.is_empty()) {
            unknown.cast::<IValueProvider>().map(Interface::into_raw)
        } else {
            Ok(std::ptr::null_mut())
        }
    }
    fn runtime(&self) -> wcore::Result<*mut SAFEARRAY> {
        let snapshot = self.current()?;
        if self.id == 1 {
            return Ok(std::ptr::null_mut());
        }
        use std::hash::{Hash, Hasher};
        let mut hash = std::hash::DefaultHasher::new();
        self.name.hash(&mut hash);
        let name = hash.finish();
        let values = [
            UiaAppendRuntimeId as i32,
            snapshot.context as i32,
            (snapshot.context >> 32) as i32,
            self.id as i32,
            name as i32,
            (name >> 32) as i32,
        ];
        // SAFETY: owned SAFEARRAY until returned to UIA; destroy on any failure.
        unsafe {
            let array = SafeArrayCreateVector(VT_I4, 0, values.len() as u32);
            if array.is_null() {
                return Err(wcore::Error::from_hresult(E_OUTOFMEMORY));
            }
            for (i, value) in values.iter().enumerate() {
                if let Err(error) =
                    SafeArrayPutElement(array, &(i as i32), value as *const i32 as *const c_void)
                {
                    let _ = SafeArrayDestroy(array);
                    return Err(error);
                }
            }
            Ok(array)
        }
    }
}
#[implement(Simple, Fragment, FragmentRoot)]
struct Root {
    element: Element,
}
#[implement(Simple, Fragment, IInvokeProvider, IValueProvider)]
struct Child {
    element: Element,
}
macro_rules! common {
    ($ty:ty) => {
        impl Simple_Impl for $ty {
            unsafe fn ProviderOptions(&self, out: *mut ProviderOptions) -> HRESULT {
                unsafe {
                    output(
                        out,
                        Ok(ProviderOptions_ServerSideProvider | ProviderOptions_UseComThreading),
                    )
                }
            }
            unsafe fn GetPatternProvider(
                &self,
                id: UIA_PATTERN_ID,
                out: *mut *mut c_void,
            ) -> HRESULT {
                unsafe {
                    output(
                        out,
                        self.element.pattern(id, self.to_interface::<IUnknown>()),
                    )
                }
            }
            unsafe fn GetPropertyValue(&self, id: UIA_PROPERTY_ID, out: *mut VARIANT) -> HRESULT {
                unsafe { output(out, self.element.property(id)) }
            }
            unsafe fn HostRawElementProvider(&self, out: *mut *mut c_void) -> HRESULT {
                unsafe {
                    output(
                        out,
                        if self.element.id == 1 {
                            UiaHostProviderFromHwnd(HWND(self.element.hwnd as *mut c_void))
                                .map(Interface::into_raw)
                        } else {
                            Ok(std::ptr::null_mut())
                        },
                    )
                }
            }
        }
        impl Fragment_Impl for $ty {
            unsafe fn Navigate(
                &self,
                direction: NavigateDirection,
                out: *mut *mut c_void,
            ) -> HRESULT {
                unsafe { output(out, self.element.navigate(direction)) }
            }
            unsafe fn GetRuntimeId(&self, out: *mut *mut SAFEARRAY) -> HRESULT {
                unsafe { output(out, self.element.runtime()) }
            }
            unsafe fn BoundingRectangle(&self, out: *mut UiaRect) -> HRESULT {
                unsafe { output(out, self.element.bounds()) }
            }
            unsafe fn GetEmbeddedFragmentRoots(&self, out: *mut *mut SAFEARRAY) -> HRESULT {
                unsafe { output(out, Ok(std::ptr::null_mut())) }
            }
            unsafe fn SetFocus(&self) -> HRESULT {
                self.element
                    .request(Action::Focus)
                    .map_or_else(|e| e.code(), |_| HRESULT(0))
            }
            unsafe fn FragmentRoot(&self, out: *mut *mut c_void) -> HRESULT {
                unsafe { output(out, self.element.root()) }
            }
        }
    };
}
common!(Root_Impl);
common!(Child_Impl);
impl FragmentRoot_Impl for Root_Impl {
    unsafe fn ElementProviderFromPoint(&self, x: f64, y: f64, out: *mut *mut c_void) -> HRESULT {
        let result = (|| {
            let snapshot = self.element.current()?;
            for node in snapshot.tree.nodes.iter().rev() {
                let fragment = self.element.fragment(node.id, snapshot.context)?;
                let bounds = unsafe { fragment.BoundingRectangle()? };
                if x >= bounds.left
                    && y >= bounds.top
                    && x < bounds.left + bounds.width
                    && y < bounds.top + bounds.height
                {
                    return Ok(fragment.into_raw());
                }
            }
            Ok(std::ptr::null_mut())
        })();
        unsafe { output(out, result) }
    }
    unsafe fn GetFocus(&self, out: *mut *mut c_void) -> HRESULT {
        let result = (|| {
            let snapshot = self.element.current()?;
            snapshot
                .tree
                .nodes
                .iter()
                .find(|n| n.focused)
                .map(|n| {
                    self.element
                        .fragment(n.id, snapshot.context)
                        .map(Interface::into_raw)
                })
                .transpose()
                .map(|v| v.unwrap_or(std::ptr::null_mut()))
        })();
        unsafe { output(out, result) }
    }
}
impl IInvokeProvider_Impl for Child_Impl {
    fn Invoke(&self) -> wcore::Result<()> {
        self.element.request(Action::Invoke)
    }
}
impl IValueProvider_Impl for Child_Impl {
    fn SetValue(&self, val: &PCWSTR) -> wcore::Result<()> {
        if val.is_null() {
            return Err(wcore::Error::from_hresult(E_POINTER));
        }
        let mut units = Vec::new();
        // SAFETY: COM supplies a NUL-terminated UTF-16 string. Bound reads and
        // allocation before constructing UTF-8; never scan an unlimited value.
        for i in 0..=MAX_TEXT {
            let unit = unsafe { val.0.add(i).read() };
            if unit == 0 {
                let text = String::from_utf16(&units)
                    .map_err(|_| wcore::Error::from_hresult(E_INVALIDARG))?;
                return self.element.request(Action::SetValue(text));
            }
            units.push(unit);
        }
        Err(wcore::Error::from_hresult(E_INVALIDARG))
    }
    fn Value(&self) -> wcore::Result<BSTR> {
        let snapshot = self.element.current()?;
        Ok(BSTR::from(self.element.node(&snapshot)?.text.as_str()))
    }
    fn IsReadOnly(&self) -> wcore::Result<BOOL> {
        let snapshot = self.element.current()?;
        Ok(BOOL::from(!self.element.node(&snapshot)?.editable))
    }
}

struct Hook {
    root: IRawElementProviderSimple,
    shared: Arc<Shared>,
    alive: Arc<AtomicBool>,
}
pub(super) struct Attachment {
    hwnd: HWND,
    hook: *mut Hook,
    alive: Arc<AtomicBool>,
    _ui_thread: PhantomData<Rc<()>>,
    _com: Apartment,
}
struct Apartment(bool);
impl Apartment {
    fn new() -> Result<Self, Error> {
        let result =
            unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE) };
        if result.is_ok() {
            Ok(Self(true))
        } else if result == RPC_E_CHANGED_MODE {
            Ok(Self(false))
        } else {
            Err(Error::Native(result.to_string()))
        }
    }
}
impl Drop for Apartment {
    fn drop(&mut self) {
        if self.0 {
            unsafe {
                CoUninitialize();
            }
        }
    }
}
const SUBCLASS_ID: usize = 0x5352434d;
impl Attachment {
    pub(super) fn new(
        identity: Option<i64>,
        title: &str,
        shared: Arc<Shared>,
    ) -> Result<Self, Error> {
        let identity = match identity {
            Some(identity) => identity,
            None => find_owned_window(title)?,
        };
        let hwnd = HWND(identity as *mut c_void);
        let mut pid = 0;
        // SAFETY: validate the borrowed HWND belongs to this process/UI thread
        // before installing our own subclass; no foreign window is modified.
        unsafe {
            let thread = GetWindowThreadProcessId(hwnd, Some(&mut pid));
            if thread != GetCurrentThreadId()
                || pid != GetCurrentProcessId()
                || !IsWindow(Some(hwnd)).as_bool()
            {
                return Err(Error::Native("window identity/thread unavailable".into()));
            }
        }
        let com = Apartment::new()?;
        let simple: Simple = Root {
            element: Element {
                shared: shared.clone(),
                hwnd: identity as isize,
                id: 1,
                context: None,
                name: None,
            },
        }
        .into();
        let root = simple.cast().map_err(|e| Error::Native(e.to_string()))?;
        let alive = Arc::new(AtomicBool::new(true));
        let hook = Box::into_raw(Box::new(Hook {
            root,
            shared,
            alive: alive.clone(),
        }));
        if !unsafe { SetWindowSubclass(hwnd, Some(subclass), SUBCLASS_ID, hook as usize) }.as_bool()
        {
            unsafe {
                drop(Box::from_raw(hook));
            }
            return Err(Error::Native("accessibility attach failed".into()));
        }
        Ok(Self {
            hwnd,
            hook,
            alive,
            _ui_thread: PhantomData,
            _com: com,
        })
    }
    pub(super) fn changed(&self) {
        if !self.alive.load(Ordering::Acquire) {
            return;
        }
        // SAFETY: UI-thread-only owner, live hook until NCDESTROY or Drop.
        unsafe {
            let hook = &*self.hook;
            let _ = UiaRaiseStructureChangedEvent(
                &hook.root,
                StructureChangeType_ChildrenInvalidated,
                std::ptr::null_mut(),
                0,
            );
        }
    }
    pub(super) fn focus(&self) {
        if self.alive.load(Ordering::Acquire) {
            unsafe {
                let _ = SetForegroundWindow(self.hwnd);
                let _ = SetFocus(Some(self.hwnd));
            }
        }
    }
}

fn find_owned_window(title: &str) -> Result<i64, Error> {
    struct Search<'a> {
        title: &'a str,
        matches: Vec<isize>,
    }
    unsafe extern "system" fn visit(hwnd: HWND, data: LPARAM) -> BOOL {
        // SAFETY: EnumThreadWindows calls synchronously with our stack data.
        let search = unsafe { &mut *(data.0 as *mut Search<'_>) };
        let mut class = [0u16; 128];
        let mut text = [0u16; 256];
        let (class_len, text_len) = unsafe {
            (
                GetClassNameW(hwnd, &mut class),
                GetWindowTextW(hwnd, &mut text),
            )
        };
        if String::from_utf16(&class[..class_len.max(0) as usize])
            .ok()
            .as_deref()
            == Some("AgenTermNativePixelWindow")
            && String::from_utf16(&text[..text_len.max(0) as usize])
                .ok()
                .as_deref()
                == Some(search.title)
        {
            search.matches.push(hwnd.0 as isize);
        }
        BOOL::from(true)
    }
    let mut search = Search {
        title,
        matches: Vec::new(),
    };
    unsafe {
        EnumThreadWindows(
            GetCurrentThreadId(),
            Some(visit),
            LPARAM(&mut search as *mut Search<'_> as isize),
        )
    }
    .ok()
    .map_err(|e| Error::Native(e.to_string()))?;
    if search.matches.len() != 1 {
        return Err(Error::Native(
            "native pixel window identity is absent or ambiguous".into(),
        ));
    }
    Ok(search.matches[0] as i64)
}
impl Drop for Attachment {
    fn drop(&mut self) {
        if self.alive.swap(false, Ordering::AcqRel) {
            unsafe {
                let _ = RemoveWindowSubclass(self.hwnd, Some(subclass), SUBCLASS_ID);
                let hook = Box::from_raw(self.hook);
                hook.shared.accepting.store(false, Ordering::Release);
                if let Ok(mut snapshot) = hook.shared.snapshot.lock() {
                    *snapshot = None;
                }
                if let Ok(mut queue) = hook.shared.queue.lock() {
                    queue.clear();
                }
                let _ = UiaReturnRawElementProvider(
                    self.hwnd,
                    WPARAM(0),
                    LPARAM(0),
                    None::<&IRawElementProviderSimple>,
                );
            }
        }
    }
}
unsafe extern "system" fn subclass(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _id: usize,
    data: usize,
) -> LRESULT {
    let hook = unsafe { &*(data as *const Hook) };
    if message == WM_GETOBJECT && lparam.0 as i32 == UiaRootObjectId {
        return unsafe { UiaReturnRawElementProvider(hwnd, wparam, lparam, &hook.root) };
    }
    if message == WM_NCDESTROY {
        hook.alive.store(false, Ordering::Release);
        hook.shared.accepting.store(false, Ordering::Release);
        if let Ok(mut snapshot) = hook.shared.snapshot.lock() {
            *snapshot = None;
        }
        if let Ok(mut queue) = hook.shared.queue.lock() {
            queue.clear();
        }
        unsafe {
            let _ = RemoveWindowSubclass(hwnd, Some(subclass), SUBCLASS_ID);
            let _ = UiaReturnRawElementProvider(
                hwnd,
                WPARAM(0),
                LPARAM(0),
                None::<&IRawElementProviderSimple>,
            );
            drop(Box::from_raw(data as *mut Hook));
        }
    }
    unsafe { DefSubclassProc(hwnd, message, wparam, lparam) }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn nullable_interfaces_match_official_abi_ids() {
        assert_eq!(Simple::IID, IRawElementProviderSimple::IID);
        assert_eq!(Fragment::IID, IRawElementProviderFragment::IID);
        assert_eq!(FragmentRoot::IID, IRawElementProviderFragmentRoot::IID);
    }
    #[test]
    fn live_field_survives_value_revision_but_not_a_reopened_form() {
        use crate::{AccessibilityBounds, PublishedTree};
        use std::{collections::VecDeque, sync::Mutex};
        let node = PublishedNode {
            id: 2,
            parent: Some(1),
            role: PublishedRole::Text,
            name: "Editable field".into(),
            text: "first".into(),
            bounds: AccessibilityBounds {
                x: 0,
                y: 0,
                width: 80,
                height: 20,
            },
            focusable: true,
            focused: true,
            editable: true,
            clickable: false,
        };
        let snapshot = Snapshot {
            revision: 1,
            context: 7,
            tree: PublishedTree {
                app_name: "Fixture".into(),
                nodes: vec![node],
            },
            disabled: vec![],
        };
        let shared = Arc::new(Shared {
            snapshot: Mutex::new(Some(Arc::new(snapshot.clone()))),
            queue: Mutex::new(VecDeque::new()),
            accepting: AtomicBool::new(true),
            wake: Arc::new(|| true),
        });
        let field = Element {
            shared: shared.clone(),
            hwnd: 0,
            id: 2,
            context: Some(7),
            name: Some("Editable field".into()),
        };
        let mut next = snapshot;
        next.revision = 2;
        next.tree.nodes[0].text = "updated".into();
        *shared.snapshot.lock().unwrap() = Some(Arc::new(next.clone()));
        let current = field.current().unwrap();
        assert_eq!(field.node(&current).unwrap().text, "updated");
        field.request(Action::SetValue("new value".into())).unwrap();
        assert_eq!(
            shared.queue.lock().unwrap().pop_front().unwrap().revision,
            2
        );
        next.context = 8;
        *shared.snapshot.lock().unwrap() = Some(Arc::new(next));
        assert!(
            field
                .request(Action::SetValue("late old form".into()))
                .is_err()
        );
        assert!(shared.queue.lock().unwrap().is_empty());
    }
}
