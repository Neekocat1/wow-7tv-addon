@echo off
rem Builds target\release\SevenTVEmotesHelper.exe and, if Inno Setup 6 is installed,
rem dist\SevenTVEmotesHelper-Setup-<version>.exe.
rem Needs Rust (https://rustup.rs, default MSVC toolchain + Visual Studio Build Tools).
cd /d "%~dp0"
cargo build --release || exit /b 1
echo.
echo Built: %~dp0target\release\SevenTVEmotesHelper.exe

set ISCC=
if exist "%ProgramFiles(x86)%\Inno Setup 6\ISCC.exe" set ISCC="%ProgramFiles(x86)%\Inno Setup 6\ISCC.exe"
if exist "%LOCALAPPDATA%\Programs\Inno Setup 6\ISCC.exe" set ISCC="%LOCALAPPDATA%\Programs\Inno Setup 6\ISCC.exe"
if not defined ISCC (
  echo Inno Setup 6 not found ^(https://jrsoftware.org/isdl.php^); skipping the installer.
  exit /b 0
)
%ISCC% /Q installer\SevenTVEmotesHelper.iss || exit /b 1
echo Built the installer in %~dp0dist\
