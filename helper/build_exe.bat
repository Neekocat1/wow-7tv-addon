@echo off
rem Builds dist\SevenTVEmotesHelper.exe (one file, no console window).
rem Needs: python -m pip install -r requirements.txt pyinstaller
cd /d "%~dp0"
python -m PyInstaller --noconfirm --onefile --windowed ^
  --name SevenTVEmotesHelper ^
  --add-data "..\addon\SevenTVEmotes;addon\SevenTVEmotes" ^
  --add-data "..\addon\SevenTVEmotes_Local;addon\SevenTVEmotes_Local" ^
  --hidden-import pystray._win32 ^
  run_helper.pyw
echo.
echo Built: %~dp0dist\SevenTVEmotesHelper.exe
echo Run it once: it replaces the running helper and the copy that starts with Windows.
