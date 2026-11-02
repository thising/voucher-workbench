@echo off
setlocal DisableDelayedExpansion
"%~dp0voucher-workbench.exe" %*
set "launch_exit=%errorlevel%"
if not "%launch_exit%"=="0" pause
exit /b %launch_exit%
