# Standing up the build VM

The dependable build host is a **Windows 10 IoT Enterprise LTSC** VM with
OpenSSH Server. If the very oldest leak tools balk at a modern OS, use a
Windows 2000 / XP VM instead (fallback section at the end).

---

## 1. Get the media and create the VM (VMware)

1. Get a Windows 10 IoT Enterprise LTSC ISO (Microsoft Evaluation Center, or
   your VLSC/partner source). LTSC 2019 (1809) or LTSC 2021 (21H2) both work.
2. In VMware Workstation/Player: **New VM → Typical**.
   - Guest OS: Windows 10 x64.
   - **CPUs:** 4   **RAM:** 8192 MB   **Disk:** 80 GB (thick is simpler).
   - Network: **NAT** (so the Linux host can reach it) — note the subnet
     (VMware default `192.168.x.0/24`) and its VMware NAT adapter IP.
   - Remove/adjust the floppy, keep BIOS (not UEFI) for simplicity.
3. Install Windows, create a local admin user `loos`, set a password.

> Do **not** enable BitLocker and do **not** join a domain.

## 2. Enable OpenSSH Server (in the VM)

PowerShell **as Administrator**:

```powershell
# capability (built into Win10; also works on IoT LTSC)
Add-WindowsCapability -Online -Name OpenSSH.Server~~~~0.0.1.0
Add-WindowsCapability -Online -Name OpenSSH.Client~~~~0.0.1.0

# start + autostart the service
Set-Service -Name sshd -StartupType Automatic
Start-Service sshd
New-NetFirewallRule -Name sshd -DisplayName 'OpenSSH Server' -Enabled True `
  -Direction Inbound -Protocol TCP -Action Allow -LocalPort 22
```

Test from the Linux host:

```bash
ssh loos@<vm-ip> "ver"
# expect: Microsoft Windows [Version 10.0.xxxxx]
```

### Optional: make ssh drop straight into cmd

By default OpenSSH on Windows launches PowerShell/cmd per the registry. Our
program always wraps commands in `cmd /c`, so this is optional, but it avoids
surprises:

```powershell
New-ItemProperty -Path "HKLM:\SOFTWARE\OpenSSH" -Name DefaultShell `
  -Value "C:\Windows\System32\cmd.exe" -PropertyType String -Force
```

### Optional: key auth (no passwords)

Standard user `loos`:
```powershell
# run in the VM
New-Item -ItemType Directory -Force -Path "$env:USERPROFILE\.ssh"
# paste your host's C:\/home/silvan/.ssh/id_ed25519.pub:
Add-Content -Path "$env:USERPROFILE\.ssh\authorized_keys" -Value '<your pubkey>'
```
For an **administrator** account the file is `C:\ProgramData\ssh\administrators_authorized_keys`
and must be ACL-restricted:
```powershell
icacls "C:\ProgramData\ssh\administrators_authorized_keys" /inheritance:r `
  /grant "SYSTEM:F" /grant "Administrators:F"
```

## 3. Prep the OS for building

```powershell
# long paths (the tree has deep dirs)
New-ItemProperty -Path "HKLM:\SYSTEM\CurrentControlSet\Control\FileSystem" `
  -Name LongPathsEnabled -Value 1 -PropertyType DWORD -Force

# never sleep
powercfg /change standby-timeout-ac 0
powercfg /change monitor-timeout-ac 0

# (optional) speed: exclude the build dir from Defender
Add-MpPreference -ExclusionPath "C:\loos"
# or disable real-time protection entirely for the VM (it is disposable):
# Set-MpPreference -DisableRealtimeMonitoring $true

# confirm tar.exe exists (needed by loos-build.py sync fallback)
where.exe tar
```

Create the working dirs:

```powershell
mkdir C:\loos | Out-Null
```

## 4. Point the host at the VM

```bash
cd /run/media/silvan/DATA/LoOS
cp remote/config.example.env remote/config.env
# set REMOTE_HOST to the VM IP, REMOTE_USER=loos, REMOTE_DIR=C:\loos\srv03rtm
python3 remote/loos-build.py sync
python3 remote/loos-build.py build
```

If the Linux host cannot reach the VM, check the VMware NAT adapter
(`ip addr` for `vmnet8`) and confirm the guest firewall allows 22.

## 5. Build time and threads

`build -bcZP -M 4` is capped at four threads because the tree is unstable above
that. A full build on 4 vCPUs is **several hours**. Give the VM 4 vCPUs and
leave it running; `loos-build.py build` streams until done.

---

## Fallback: Windows 2000 / XP VM

Only needed if modern Win10 rejects an old tool. XP/2000 have no built-in SSH,
so use either OpenSSH-Win32 or a shared folder.

**Option A — Win32-OpenSSH** (works on XP/2003 era with the older v7.x builds;
2000 is unreliable): copy the release, run `Install-sshd.ps1` (or the manual
service install), start `sshd`.

**Option B — shared folder (simplest for XP):**

1. On the Linux host, share the tree over SMB (Samba) or use VMware Shared
   Folders (requires VMware Tools in the guest).
2. In the XP VM: `net use Z: \\linux-host\srv03rtm`.
3. Build in place from a razzle prompt in `Z:` (the tree's scripts assume a
   drive letter; `\` paths work).

Because `loos-build.py` speaks plain `ssh`/`scp`, the same program works with
the XP VM once `sshd` is up; if you use a share instead, drive the build
manually with `remote/build-in-vm.cmd` copied into the tree and run under
razzle on the guest.

> The XP/2000 path is a fallback. Its value is that it matches the era of the
> leak; its cost is that the toolchain and OpenSSH setup are fragile. Prefer
> the Win10 IoT LTSC VM.
