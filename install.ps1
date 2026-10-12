# Builds the plugin and installs it into OpenDeck's plugin directory.
# Run with OpenDeck closed (otherwise the plugin .exe is locked).
#   .\install.ps1            language of the action names = Windows display language (pl / en)
#   .\install.ps1 -Lang en   force English (or -Lang pl)
param(
    [ValidateSet('en', 'pl')]
    [string]$Lang = $(if ((Get-UICulture).TwoLetterISOLanguageName -eq 'pl') { 'pl' } else { 'en' })
)

Set-Location $PSScriptRoot

# cargo writes progress to stderr, so don't treat stderr as an error here.
cmd /c "cargo build --release 2>&1"
if ($LASTEXITCODE -ne 0) { throw 'Build failed' }

python package.py --install --lang $Lang
if ($LASTEXITCODE -ne 0) { throw 'Install failed' }
