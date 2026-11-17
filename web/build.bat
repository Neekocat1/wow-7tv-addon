@echo off
rem Builds the pack builder's WebAssembly module into web\site.
rem Needs Rust with the WebAssembly target: rustup target add wasm32-unknown-unknown
cd /d "%~dp0"
cargo build --release --target wasm32-unknown-unknown --manifest-path wasm\Cargo.toml || exit /b 1
copy /y wasm\target\wasm32-unknown-unknown\release\seventv_pack_wasm.wasm site\pack.wasm >nul
for /f "tokens=3 delims= " %%v in ('findstr /b /c:"version = " wasm\Cargo.toml') do (echo %%~v> site\version.txt & goto done)
:done
echo Built web\site\pack.wasm
