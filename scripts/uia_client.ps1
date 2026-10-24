param([long]$Handle,[int]$OwnerPid,[string]$RequestFile,[string]$OutputFile)
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName UIAutomationClient
Add-Type -AssemblyName UIAutomationTypes
$request = [IO.File]::ReadAllText($RequestFile) | ConvertFrom-Json
$root = [System.Windows.Automation.AutomationElement]::FromHandle([IntPtr]$Handle)
if ($root.Current.ProcessId -ne $OwnerPid) {
    $identity = @{ actual_pid = $root.Current.ProcessId; expected_pid = $OwnerPid; name = $root.Current.Name; native_window = $root.Current.NativeWindowHandle }
    [IO.File]::WriteAllText($OutputFile, ($identity | ConvertTo-Json), [Text.UTF8Encoding]::new($false))
    throw "UIA root is not the owned product process: actual=$($identity.actual_pid) expected=$OwnerPid"
}
function Find-Node([string]$name) {
    $condition = New-Object System.Windows.Automation.PropertyCondition([System.Windows.Automation.AutomationElement]::NameProperty, $name)
    $node = $root.FindFirst([System.Windows.Automation.TreeScope]::Descendants, $condition)
    if ($null -eq $node) { throw "Missing native UIA node: $name" }
    return $node
}
function Ready-Node([string]$name) {
    $until = [DateTime]::UtcNow.AddSeconds(10)
    $started = [DateTime]::UtcNow
    $script:readiness = @{ name = $name; polls = 0; initial = $null }
    do {
        $script:readiness.polls++
        try {
            $node = Find-Node $name
            $state = @{ name = $node.Current.Name; enabled = $node.Current.IsEnabled; automation_id = $node.Current.AutomationId }
            if ($null -eq $script:readiness.initial) { $script:readiness.initial = $state }
            $script:readiness.last = $state
            if ($state.enabled) {
                $script:readiness.wait_ms = ([DateTime]::UtcNow - $started).TotalMilliseconds
                return $node
            }
        } catch { $script:readiness.last_error = $_.Exception.Message }
        Start-Sleep -Milliseconds 50
    } while ([DateTime]::UtcNow -lt $until)
    throw "Native control did not become enabled: $name"
}
function Invoke-Node($node) {
    $pattern = $node.GetCurrentPattern([System.Windows.Automation.InvokePattern]::Pattern)
    $pattern.Invoke()
}
$result = @{ mode = $request.mode; root_name = $root.Current.Name; owner = $OwnerPid }
try {
switch ($request.mode) {
    'list' {
        $nodes = $root.FindAll([System.Windows.Automation.TreeScope]::Descendants, [System.Windows.Automation.Condition]::TrueCondition)
        $result.nodes = @($nodes | ForEach-Object {
            $value = $null
            $pattern = $null
            if ($_.TryGetCurrentPattern([System.Windows.Automation.ValuePattern]::Pattern, [ref]$pattern)) { $value = $pattern.Current.Value }
            @{ name = $_.Current.Name; enabled = $_.Current.IsEnabled; role = $_.Current.ControlType.ProgrammaticName; value = $value; automation_id = $_.Current.AutomationId }
        })
    }
    'invoke' { $target = Ready-Node $request.name; $result.readiness = $script:readiness; Invoke-Node $target; $result.verdict = 'QUEUED' }
    'set' {
        $target = Ready-Node $request.name
        $result.readiness = $script:readiness
        $pattern = $target.GetCurrentPattern([System.Windows.Automation.ValuePattern]::Pattern)
        $pattern.SetValue($request.value)
        $result.verdict = 'QUEUED'
    }
    'disabled' {
        $node = Find-Node $request.name
        if ($node.Current.IsEnabled) { throw 'Expected an actually disabled UIA node' }
        try { Invoke-Node $node; throw 'Disabled UIA invocation was accepted' }
        catch [System.Windows.Automation.ElementNotEnabledException] { $result.verdict = 'REFUSED_DISABLED' }
    }
    'stale' {
        $old = Find-Node $request.name
        $oldPattern = $old.GetCurrentPattern([System.Windows.Automation.InvokePattern]::Pattern)
        Invoke-Node (Find-Node $request.next)
        $until = [DateTime]::UtcNow.AddSeconds(15)
        do {
            try { $null = Find-Node $request.after; break }
            catch { if ([DateTime]::UtcNow -ge $until) { throw }; Start-Sleep -Milliseconds 50 }
        } while ($true)
        try { $oldPattern.Invoke(); throw 'Stale UIA invocation was accepted' }
        catch [System.Windows.Automation.ElementNotAvailableException] { $result.verdict = 'REFUSED_STALE' }
    }
    default { throw 'Unsupported UIA probe mode' }
}
} catch {
    $result.verdict = 'FAIL'
    $result.error_type = $_.Exception.GetType().FullName
    $result.error = $_.Exception.Message
    if ($null -ne $script:readiness) { $result.readiness = $script:readiness }
    [IO.File]::WriteAllText($OutputFile, ($result | ConvertTo-Json -Depth 8), [Text.UTF8Encoding]::new($false))
    throw
}
[IO.File]::WriteAllText($OutputFile, ($result | ConvertTo-Json -Depth 8), [Text.UTF8Encoding]::new($false))
