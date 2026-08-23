[CmdletBinding()]
param(
    [ValidateSet('x64')]
    [string] $Target = 'x64',
    [ValidateSet('Debug', 'RelWithDebInfo', 'Release', 'MinSizeRel')]
    [string] $Configuration = 'RelWithDebInfo',
    [switch] $BuildInstaller
)

$ErrorActionPreference = 'Stop'

if ( $DebugPreference -eq 'Continue' ) {
    $VerbosePreference = 'Continue'
    $InformationPreference = 'Continue'
}

if ( $env:CI -eq $null ) {
    throw "Package-Windows.ps1 requires CI environment"
}

if ( ! ( [System.Environment]::Is64BitOperatingSystem ) ) {
    throw "Packaging script requires a 64-bit system to build and run."
}

if ( $PSVersionTable.PSVersion -lt '7.2.0' ) {
    Write-Warning 'The packaging script requires PowerShell Core 7. Install or upgrade your PowerShell version: https://aka.ms/pscore6'
    exit 2
}

# Inno Setup is preinstalled on GitHub's Windows runners, but not always on the
# PATH, so fall back to the places its installer puts it.
function Find-InnoSetup {
    $Command = Get-Command -Name 'iscc' -CommandType Application -ErrorAction SilentlyContinue

    if ( $Command ) {
        return $Command[0].Source
    }

    $Candidates = @(
        "${Env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe"
        "${Env:ProgramFiles}\Inno Setup 6\ISCC.exe"
    )

    foreach ( $Candidate in $Candidates ) {
        if ( Test-Path -Path $Candidate ) {
            return $Candidate
        }
    }

    throw 'Inno Setup (ISCC.exe) not found. Install it from https://jrsoftware.org/isinfo.php to build the installer.'
}

function Package {
    trap {
        Write-Error $_
        exit 2
    }

    $ScriptHome = $PSScriptRoot
    $ProjectRoot = Resolve-Path -Path "$PSScriptRoot/../.."
    $BuildSpecFile = "${ProjectRoot}/buildspec.json"

    $UtilityFunctions = Get-ChildItem -Path $PSScriptRoot/utils.pwsh/*.ps1 -Recurse

    foreach( $Utility in $UtilityFunctions ) {
        Write-Debug "Loading $($Utility.FullName)"
        . $Utility.FullName
    }

    $BuildSpec = Get-Content -Path ${BuildSpecFile} -Raw | ConvertFrom-Json
    $ProductName = $BuildSpec.name
    $ProductVersion = $BuildSpec.version

    $OutputName = "${ProductName}-${ProductVersion}-windows-${Target}"

    $RemoveArgs = @{
        ErrorAction = 'SilentlyContinue'
        Path = @(
            "${ProjectRoot}/release/${ProductName}-*-windows-*.zip"
            "${ProjectRoot}/release/${ProductName}-*-windows-*-Installer.exe"
        )
    }

    Remove-Item @RemoveArgs

    if ( $BuildInstaller ) {
        $Iscc = Find-InnoSetup
    }

    Log-Group "Archiving ${ProductName}..."
    $CompressArgs = @{
        Path = (Get-ChildItem -Path "${ProjectRoot}/release/${Configuration}" -Exclude "${OutputName}*.*")
        CompressionLevel = 'Optimal'
        DestinationPath = "${ProjectRoot}/release/${OutputName}.zip"
        Verbose = ($Env:CI -ne $null)
    }
    Compress-Archive -Force @CompressArgs
    Log-Group

    if ( $BuildInstaller ) {
        Log-Group "Building ${ProductName} installer..."

        $IssFile = "${ProjectRoot}/build_${Target}/installer-Windows.generated.iss"

        if ( ! ( Test-Path -Path $IssFile ) ) {
            throw "Inno Setup script not found at ${IssFile}. Configure the project before packaging."
        }

        # Inno Setup only understands backslash-separated paths.
        $StageDir = (Convert-Path "${ProjectRoot}/release/${Configuration}")
        $OutputDir = (Convert-Path "${ProjectRoot}/release")

        $IsccArgs = @(
            (Convert-Path $IssFile)
            "/DReleaseDir=${StageDir}"
            "/O${OutputDir}"
            "/F${OutputName}-Installer"
        )

        Invoke-External $Iscc @IsccArgs
        Log-Group
    }
}

Package
