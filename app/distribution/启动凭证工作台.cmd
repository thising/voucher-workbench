@echo off
setlocal DisableDelayedExpansion
chcp 65001 >nul
cd /d "%~dp0"
echo 凭证工作台将在浏览器中打开。请保留此窗口，结束时按 Ctrl+C。
echo 关闭网页不会停止后台任务。
"%~dp0voucher-workbench.exe"
if errorlevel 1 (
  echo.
  echo 程序启动或运行失败，请查看上方错误说明。
  echo 如果提示已有服务，请使用原服务窗口中的链接。
  pause
)
endlocal
