@echo off
rem Zips the SevenTVEmotes addon for CurseForge into ..\dist\SevenTVEmotes-<version>.zip.
rem Only SevenTVEmotes: SevenTVEmotes_Local belongs to the helper / emote packs, and CurseForge
rem would wipe players' emotes if it managed that folder.
cd /d "%~dp0"
for /f "tokens=3" %%v in ('findstr /b /c:"## Version:" SevenTVEmotes\SevenTVEmotes.toc') do set VER=%%v
if not exist ..\dist mkdir ..\dist
del /q "..\dist\SevenTVEmotes-%VER%.zip" 2>nul
tar -a -c -f "..\dist\SevenTVEmotes-%VER%.zip" --exclude=SevenTVEmotes/emotes SevenTVEmotes || exit /b 1
echo Built dist\SevenTVEmotes-%VER%.zip
