@echo off
rem Windows: run or build the fork.
rem
rem   scripts\fork\app.cmd dev      run in development mode
rem   scripts\fork\app.cmd build    installer (.msi and setup .exe)
rem
rem Does what upstream's frontend\build-gpu.bat / dev-gpu.bat do (GPU feature
rem detection, llama-helper sidecar, tauri build), but sets up Visual Studio
rem with vcvars64.bat alone, so any installed Windows SDK works. The upstream
rem scripts hardcode Windows SDK 10.0.22621 and MSVC 14.44 paths.
rem The fork's updater settings are in frontend\src-tauri\tauri.windows.conf.json.
setlocal enabledelayedexpansion
set "MODE=%~1"
if "%MODE%"=="" set "MODE=dev"
if /i not "%MODE%"=="dev" if /i not "%MODE%"=="build" (
  echo usage: scripts\fork\app.cmd dev^|build
  exit /b 1
)
set "ROOT=%~dp0..\.."
cd /d "%ROOT%\frontend" || exit /b 1

rem --- prerequisites -------------------------------------------------------
if not exist "C:\Program Files\LLVM\bin\libclang.dll" (
  echo LLVM not found at C:\Program Files\LLVM. Install it: winget install LLVM.LLVM
  exit /b 1
)
set "LIBCLANG_PATH=C:\Program Files\LLVM\bin"
rem whisper-rs-sys (bindgen 0.69) misreads whisper.h with LLVM 20 or newer and
rem produces empty structs ("no field `greedy` on type `whisper_full_params`").
rem Require LLVM 19 or older.
set "CLANG_MAJOR="
for /f "tokens=3 delims= " %%v in ('"C:\Program Files\LLVM\bin\clang.exe" --version ^| findstr /b clang') do (
  for /f "tokens=1 delims=." %%m in ("%%v") do set "CLANG_MAJOR=%%m"
)
if defined CLANG_MAJOR if !CLANG_MAJOR! GEQ 20 (
  echo LLVM !CLANG_MAJOR! is too new for Motiv's Whisper bindings. Install LLVM 18:
  echo   winget uninstall LLVM.LLVM
  echo   winget install LLVM.LLVM --version 18.1.8
  echo then: cd "%ROOT%" ^&^& cargo clean -p whisper-rs-sys --release
  exit /b 1
)
echo LLVM !CLANG_MAJOR!

set "VSWHERE=%ProgramFiles(x86)%\Microsoft Visual Studio\Installer\vswhere.exe"
if not exist "%VSWHERE%" (
  echo Visual Studio Build Tools not found. See FORK.md / docs\BUILDING.md.
  exit /b 1
)
set "VSPATH="
for /f "usebackq delims=" %%i in (`"%VSWHERE%" -latest -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath`) do set "VSPATH=%%i"
if not defined VSPATH (
  echo Visual Studio C++ build tools not found. Install the "Desktop development with C++" workload.
  exit /b 1
)
echo Using Visual Studio at !VSPATH!
call "!VSPATH!\VC\Auxiliary\Build\vcvars64.bat" >nul || exit /b 1
echo Windows SDK: %WindowsSDKVersion%

where pnpm >nul 2>&1 || (echo pnpm not found. Run: npm install -g pnpm@9.15.9 & exit /b 1)
if not exist node_modules (
  call pnpm install --frozen-lockfile || exit /b 1
)

rem --- GPU feature (same detection as upstream) ---------------------------
set "TAURI_GPU_FEATURE="
for /f "delims=" %%i in ('node scripts\auto-detect-gpu.js') do set "TAURI_GPU_FEATURE=%%i"
if "!TAURI_GPU_FEATURE!"=="none" set "TAURI_GPU_FEATURE="
set "HELPER_FEATURES="
if defined TAURI_GPU_FEATURE set "HELPER_FEATURES=--features !TAURI_GPU_FEATURE!"
echo GPU feature: !TAURI_GPU_FEATURE! (empty = CPU)

rem --- llama-helper sidecar ------------------------------------------------
set "PROFILE=debug"
set "RELEASE_FLAG="
if /i "%MODE%"=="build" (
  set "PROFILE=release"
  set "RELEASE_FLAG=--release"
)
echo Building llama-helper (!PROFILE!) !HELPER_FEATURES!
pushd "%ROOT%\llama-helper" || exit /b 1
cargo build !RELEASE_FLAG! !HELPER_FEATURES!
if errorlevel 1 (
  popd
  echo Failed to build llama-helper
  exit /b 1
)
popd
for /f "tokens=2" %%i in ('rustc -vV ^| findstr "host:"') do set "TRIPLE=%%i"
if not exist src-tauri\binaries mkdir src-tauri\binaries
del /q src-tauri\binaries\llama-helper* 2>nul
copy /Y "%ROOT%\target\!PROFILE!\llama-helper.exe" "src-tauri\binaries\llama-helper-!TRIPLE!.exe" >nul || (
  echo llama-helper.exe not found in target\!PROFILE!
  exit /b 1
)

rem --- run or build ---------------------------------------------------------
if /i "%MODE%"=="dev" (
  call pnpm run tauri:dev
) else (
  call pnpm run tauri:build
  if not errorlevel 1 (
    echo.
    echo Installers are in:
    echo   %ROOT%\target\release\bundle\msi
    echo   %ROOT%\target\release\bundle\nsis
  )
)
