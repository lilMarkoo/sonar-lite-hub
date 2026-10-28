@echo off
where cargo >nul 2>nul
if errorlevel 1 (
  echo Rust is not installed. Get it from https://rustup.rs and run this again.
  exit /b 1
)
cargo build --release
if errorlevel 1 exit /b 1
copy /Y target\release\sonar-lite.exe SonarLite.exe >nul
echo.
echo Done: SonarLite.exe
