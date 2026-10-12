@echo off
cd /d "%~dp0"
deviceinstaller64.exe install usbmmidd.inf usbmmidd
deviceinstaller64.exe enableidd 1
exit /b 0
