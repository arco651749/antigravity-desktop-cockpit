@echo off
cd /d "%~dp0"
echo [1/2] Installing Amyuni/RustDesk Virtual Display Driver...
deviceinstaller64.exe install usbmmidd.inf usbmmidd
echo [2/2] Enabling Virtual Monitor 1...
deviceinstaller64.exe enableidd 1
echo Done!
