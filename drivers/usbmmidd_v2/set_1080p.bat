@echo off
cd /d "%~dp0"
echo [1/3] Setting default virtual monitor resolution to 1920x1080...
reg add "HKLM\SOFTWARE\Microsoft\Windows NT\CurrentVersion\WUDF\Services\usbmmIdd\Parameters\Monitors" /v "0" /t REG_SZ /d "1920,1080" /f
echo [2/3] Cycling virtual monitor...
deviceinstaller64.exe enableidd 0
deviceinstaller64.exe enableidd 1
echo [3/3] Successfully applied 1920x1080!
timeout /t 3
