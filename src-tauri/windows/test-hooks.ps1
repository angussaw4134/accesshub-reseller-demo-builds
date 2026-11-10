param([string]$NsisCompiler = (Join-Path $env:LOCALAPPDATA 'tauri\NSIS\makensis.exe'))
$ErrorActionPreference = 'Stop'
if (-not (Test-Path -LiteralPath $NsisCompiler)) { throw 'Build the Windows installer first to prepare NSIS.' }
$testDirectory = Join-Path $PSScriptRoot ('..\target\nsis-hook-checks\' + [Guid]::NewGuid().ToString('N'))
$null = New-Item -ItemType Directory -Path $testDirectory -Force
$testDirectory = (Resolve-Path -LiteralPath $testDirectory).Path
$outputDirectory = Join-Path $testDirectory 'bootstrap'
$null = New-Item -ItemType Directory -Path $outputDirectory
$hookFile = Join-Path $PSScriptRoot 'hooks.nsh'
$harness = Join-Path $testDirectory 'harness.nsi'
$executable = Join-Path $testDirectory 'hook-test.exe'
$script = @"
Unicode true
Name "AccessHub installer handoff check"
OutFile "$executable"
RequestExecutionLevel user
SilentInstall silent
AutoCloseWindow true
!define ACCESSHUB_BOOTSTRAP_DIRECTORY "$outputDirectory"
!include "$hookFile"
Section
  !insertmacro NSIS_HOOK_POSTINSTALL
  SetErrorLevel 0
SectionEnd
"@
[IO.File]::WriteAllText($harness, $script, [Text.UTF8Encoding]::new($false))
& $NsisCompiler /V2 $harness
if ($LASTEXITCODE -ne 0) { throw 'NSIS handoff test did not compile.' }
$ticket = ('A' * 41) + '_-'
$ticketFile = Join-Path $outputDirectory 'activation-ticket'
$cases = @(
    @{ Name = "AccessHub-Setup--$ticket.exe"; Valid = $true },
    @{ Name = "AccessHub-Setup--$ticket (1).exe"; Valid = $true },
    @{ Name = "AccessHub-Setup--$ticket(23).exe"; Valid = $true },
    @{ Name = 'AccessHub_1.0.0-beta.4_x64-setup.exe'; Valid = $false },
    @{ Name = "Other-Setup--$ticket.exe"; Valid = $false },
    @{ Name = "AccessHub-Setup--$($ticket.Substring(1)).exe"; Valid = $false },
    @{ Name = "AccessHub-Setup--$ticket ().exe"; Valid = $false },
    @{ Name = "AccessHub-Setup--$ticket (x).exe"; Valid = $false },
    @{ Name = "AccessHub-Setup--$ticket-extra.exe"; Valid = $false },
    @{ Name = "AccessHub-Setup--$('A' * 42)!.exe"; Valid = $false }
)
foreach ($case in $cases) {
    $renamed = Join-Path $testDirectory $case.Name
    Copy-Item -LiteralPath $executable -Destination $renamed
    [IO.File]::WriteAllText($ticketFile, 'leave-existing-bootstrap', [Text.Encoding]::ASCII)
    $process = Start-Process -FilePath $renamed -WindowStyle Hidden -Wait -PassThru
    if ($process.ExitCode -ne 0) { throw 'NSIS handoff check returned an error.' }
    $expected = if ($case.Valid) { $ticket } else { 'leave-existing-bootstrap' }
    if ([IO.File]::ReadAllText($ticketFile) -cne $expected) { throw 'NSIS filename handoff check failed.' }
}
Write-Output "NSIS filename handoff: $($cases.Count) checks passed."
