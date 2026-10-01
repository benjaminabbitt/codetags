# Runs a command de-elevated on a GitHub-hosted Windows runner (V18).
#
# Hosted runners run jobs elevated, and UAC is off, so there is no filtered
# token to fall back to. `runas /trustlevel:0x20000` (the SAFER "Basic User"
# level) starts a process as the same user with BUILTIN\Administrators made
# deny-only and the admin privileges removed, so `net session` fails, as it
# does for an unelevated user. Same user, so the toolchain and caches under
# the profile stay readable.
#
# runas returns at once and gives its child the user's default environment,
# not this step's, so this script saves the environment, has the child
# restore it, waits for the child, prints its output, and exits with its exit
# status.
#
# Usage: ci/run-deelevated.ps1 <program> [<argument>...]
# e.g. `ci/run-deelevated.ps1 just test-mount`, run from the directory the
# program should run in.
$ErrorActionPreference = 'Stop'
# $args, not a ValueFromRemainingArguments parameter: binding that to
# [string[]] joined the arguments into one string.
[string[]] $Command = @($args)
if ($Command.Count -eq 0) { throw 'usage: ci/run-deelevated.ps1 <program> [<argument>...]' }

$dir = Join-Path $env:RUNNER_TEMP ("deelevated-" + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $dir | Out-Null
$envFile = Join-Path $dir 'env.clixml'
$log = Join-Path $dir 'output.log'
$status = Join-Path $dir 'status.txt'
$argsFile = Join-Path $dir 'args.clixml'
$child = Join-Path $dir 'child.ps1'

Get-ChildItem env: | Select-Object Name, Value | Export-Clixml -Path $envFile
# Piped, so each argument is its own object and Import-Clixml returns a list.
$Command | Export-Clixml -Path $argsFile
$commandLine = $Command -join ' '
$cwd = (Get-Location).Path

@"
`$ErrorActionPreference = 'Continue'
Import-Clixml -Path '$envFile' | ForEach-Object {
    [Environment]::SetEnvironmentVariable(`$_.Name, `$_.Value, 'Process')
}
Set-Location -LiteralPath '$cwd'
"run-deelevated: started as `$(whoami)" | Add-Content -Path '$log'
`$code = 1
try {
    `$argv = @(Import-Clixml -Path '$argsFile')
    `$rest = @(`$argv | Select-Object -Skip 1)
    & `$argv[0] @rest *>> '$log'
    `$code = `$LASTEXITCODE
} catch {
    `$_ | Out-String | Add-Content -Path '$log'
} finally {
    Set-Content -Path '$status' -Value `$code
}
"@ | Set-Content -Path $child -Encoding utf8

# runas takes one command line; keep it free of quotes. pwsh is on the
# machine PATH, which the child's default environment has.
if ($child -match '\s') { throw "run-deelevated: $child contains whitespace" }
Write-Host "run-deelevated: $commandLine"
runas /trustlevel:0x20000 "pwsh -NoProfile -NonInteractive -File $child"
if ($LASTEXITCODE -ne 0) {
    Write-Host "run-deelevated: runas failed with exit status $LASTEXITCODE"
    exit 1
}

# Stream the child's output while it runs. The child writes its log at once;
# if none appears, it never started.
$printed = 0
$started = Get-Date
while ($true) {
    if (-not (Test-Path $log) -and ((Get-Date) - $started).TotalSeconds -gt 120) {
        Write-Host 'run-deelevated: the child process did not start'
        exit 1
    }
    $done = Test-Path $status
    if (Test-Path $log) {
        $lines = @(Get-Content -Path $log)
        if ($lines.Count -gt $printed) {
            $lines[$printed..($lines.Count - 1)] | ForEach-Object { Write-Host $_ }
            $printed = $lines.Count
        }
    }
    if ($done) { break }
    Start-Sleep -Seconds 2
}
$code = [int](Get-Content -Path $status -Raw).Trim()
Write-Host "run-deelevated: exit status $code"
exit $code
