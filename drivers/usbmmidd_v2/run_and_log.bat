@echo off
cd /d "d:\AntiGravity\Agent-Study\antigravity-desktop-cockpit\drivers\usbmmidd_v2"
echo === STARTING INSTALL === > "d:\AntiGravity\enable_log.txt"
deviceinstaller64.exe install usbmmidd.inf usbmmidd >> "d:\AntiGravity\enable_log.txt" 2>&1
echo === INSTALL DONE rc=%errorlevel% === >> "d:\AntiGravity\enable_log.txt"
deviceinstaller64.exe enableidd 1 >> "d:\AntiGravity\enable_log.txt" 2>&1
echo === ENABLE DONE rc=%errorlevel% === >> "d:\AntiGravity\enable_log.txt"
exit /b 0
