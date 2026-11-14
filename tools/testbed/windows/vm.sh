#!/bin/bash
# Orither's Windows 11 test machine: Microsoft's evaluation ISO, installed unattended into a QEMU/KVM
# virtual machine (autounattend.xml), with a small agent (agent.ps1) that runs PowerShell sent from
# here in the logged-on session, so windows it opens are on the desktop.
#
#   vm.sh iso                download the Windows 11 Enterprise evaluation ISO (about 8 GB)
#   vm.sh install            create the disk and install Windows (about 20 minutes, unattended)
#   vm.sh start | stop       start the installed machine, or shut it down
#   vm.sh serve              the agent's server and file share (run it in the background)
#   vm.sh run <powershell>   runs PowerShell in the machine and prints what it printed
#   vm.sh ui <powershell>    the same, with Find-UI / Click-UI / Get-Windows / Get-Texts loaded
#   vm.sh click <x> <y>      clicks at screen pixels (1280x800)
#   vm.sh key <keys...>      presses keys (QEMU names: ret, esc, shift-f10…)
#   vm.sh shot <file.png>    a screenshot
#
# Files put in $STATE/files can be fetched in the machine from http://10.0.2.2:8800/files/<name>.
# The host is 10.0.2.2 from inside; an adb client there reaches the test phone's adb server with
# ADB_SERVER_SOCKET=tcp:10.0.2.2:5040.
HERE=$(cd "$(dirname "$0")" && pwd)
STATE=${ORITHER_TESTBED:-$HOME/.cache/orither-testbed}/windows
MON=$STATE/monitor.sock; QMP=$STATE/qmp.sock
ISO_URL='https://go.microsoft.com/fwlink/?LinkId=2382600&clcid=0x409&culture=en-us&country=us'
mkdir -p "$STATE/files"
mon() { printf '%s\n' "$1" | socat - "UNIX-CONNECT:$MON" > /dev/null; }
boot() {
  qemu-system-x86_64 -name orither-win -machine q35,accel=kvm -cpu host,hv_relaxed,hv_vapic,hv_spinlocks=0x1fff,hv_time \
    -smp 6 -m 8192 \
    -drive if=pflash,format=raw,readonly=on,file=/usr/share/edk2/x64/OVMF_CODE.4m.fd \
    -drive if=pflash,format=raw,file="$STATE/OVMF_VARS.fd" \
    -drive file="$STATE/win11.img",format=raw,if=none,id=disk,cache=unsafe,discard=unmap -device ide-hd,drive=disk,bus=ide.0,bootindex=1 \
    -device qemu-xhci,id=xhci -device usb-tablet,bus=xhci.0 -nic user,model=e1000e \
    -vga std -display none -monitor unix:$MON,server,nowait -qmp unix:$QMP,server,nowait \
    "$@" -daemonize -pidfile "$STATE/qemu.pid"
}
case "$1" in
iso) curl -L -C - -o "$STATE/win11-eval.iso" "$ISO_URL" ;;
install)
  [ -f "$STATE/win11-eval.iso" ] || { echo "run vm.sh iso first"; exit 1; }
  truncate -s 64G "$STATE/win11.img"; cp /usr/share/edk2/x64/OVMF_VARS.4m.fd "$STATE/OVMF_VARS.fd"
  rm -f "$STATE/unattend.img"; mkfs.fat -C -n ORITHERTEST "$STATE/unattend.img" 32768 > /dev/null
  mcopy -i "$STATE/unattend.img" "$HERE/autounattend.xml" "$HERE/agent.ps1" "$HERE/agent.vbs" ::/
  # A removable drive: Windows Setup ignores answer files on fixed disks without a partition table.
  boot -drive file="$STATE/win11-eval.iso",media=cdrom,if=none,id=cd -device ide-cd,drive=cd,bus=ide.1,bootindex=0 \
    -drive file="$STATE/unattend.img",format=raw,if=none,id=ua -device usb-storage,drive=ua,bus=xhci.0,removable=on
  # "Press any key to boot from CD or DVD".
  for _ in $(seq 1 16); do sleep 1; mon "sendkey spc"; done
  echo "Installing. The agent answers once Windows has logged on: vm.sh run hostname"
  ;;
start) boot ;;
stop) mon system_powerdown ;;
serve) ORITHER_VM_FILES="$STATE/files" exec python3 -I "$HERE/server.py" ;;
run|ui)
  script="$2"
  [ "$1" = ui ] && script="iwr -UseBasicParsing http://10.0.2.2:8800/files/ui.ps1 -OutFile C:\\agent\\ui.ps1; . C:\\agent\\ui.ps1; $script" && cp "$HERE/ui.ps1" "$STATE/files/"
  python3 -I - "$script" <<'PY'
import json, sys, urllib.request
def call(path, data=None, timeout=None):
    req = urllib.request.Request("http://127.0.0.1:8800" + path, data=json.dumps(data).encode() if data is not None else None,
                                 headers={"Content-Type": "application/json"})
    return json.load(urllib.request.urlopen(req, timeout=timeout))
job = call("/enqueue", {"script": sys.argv[1], "timeout": 300})
result = call("/wait/" + job["id"], timeout=360)
sys.stdout.write(result["output"]); sys.exit(0 if result["code"] == 0 else 1)
PY
  ;;
click)
  python3 -I - "$QMP" "$2" "$3" <<'PY'
import json, socket, sys, time
s = socket.socket(socket.AF_UNIX); s.connect(sys.argv[1]); f = s.makefile("rw")
def cmd(c, **a):
    f.write(json.dumps({"execute": c, "arguments": a} if a else {"execute": c}) + "\n"); f.flush()
    while True:
        r = json.loads(f.readline())
        if "return" in r or "error" in r: return r
f.readline(); cmd("qmp_capabilities")
x, y = int(sys.argv[2]), int(sys.argv[3])
cmd("input-send-event", events=[{"type": "abs", "data": {"axis": "x", "value": x * 32767 // 1280}}, {"type": "abs", "data": {"axis": "y", "value": y * 32767 // 800}}])
time.sleep(0.15)
for down in (True, False):
    cmd("input-send-event", events=[{"type": "btn", "data": {"down": down, "button": "left"}}]); time.sleep(0.08)
PY
  ;;
key) shift; for k in "$@"; do mon "sendkey $k"; sleep 0.15; done ;;
shot) mon "screendump $STATE/screen.ppm" && sleep 0.5 && magick "$STATE/screen.ppm" "$2" ;;
*) sed -n '2,20p' "$0"; exit 1 ;;
esac
