# campus-network-autologin

Lightweight Dr.COM campus network auto-login tool written in Rust.

## What it does

- Saves credentials to `campus-network/config.toml` under the platform config directory
  - Linux: `~/.config/campus-network/config.toml`
  - Windows: `%APPDATA%\campus-network\config.toml`
- Opens a ratatui-based terminal setup screen when run without arguments
- Supports CLI-based config updates when run with arguments
- Reproduces the observed HFUT Xuancheng Dr.COM password transform
- Only attempts campus authentication when the configured campus gateway looks reachable
- Detects captive-portal redirects and re-authenticates automatically
- Sleeps between checks to keep CPU usage negligible once the network is online

## Commands

```bash
# Interactive setup (no arguments)
campus-network-autologin

# Save config from the CLI
campus-network-autologin config set \
  --username 2022xxxxxx \
  --password your-password \
  --portal-url http://172.18.3.3/0.htm \
  --probe-url http://www.baidu.com/ \
  --campus-gateways 172.18.3.3,172.18.2.2

# Show the redacted config
campus-network-autologin config show

# Submit one login attempt and verify internet access
campus-network-autologin login

# Check whether the network is online or captive
campus-network-autologin status

# Run the low-overhead monitor
campus-network-autologin run

# Print the config path
campus-network-autologin paths

# Install/remove user autostart entry for this executable
campus-network-autologin autostart install
campus-network-autologin autostart remove
```

## Config file

```toml
[auth]
username = "2022xxxxxx"
password = "plain-text-for-now"
portal_url = "http://172.18.3.3/0.htm"

[detect]
probe_url = "http://www.baidu.com/"
request_timeout_secs = 8

[daemon]
online_check_interval_secs = 300

[campus]
ipv4_cidrs = []
gateway_hosts = ["172.18.3.3", "172.18.2.2"]
```

The login form is submitted as:

```text
DDDDD=<username>
upass=<md5("2" + password + "12345678") + "123456782">
R1=0
R2=1
para=00
0MKKey=123456
v6ip=
```

## TUI controls

- `Tab` / `Shift+Tab`: move between fields and buttons
- `Enter`: jump to the next field or activate the selected button
- `Ctrl+S`: save config
- `Ctrl+T`: save config and test login immediately
- `Ctrl+A`: toggle current-user autostart
- `F2`: show or hide the password
- `Esc`: quit without saving further changes

Use the `Autostart: ON/OFF` button in TUI to toggle current-user autostart.
You can toggle it by:
- selecting that button and pressing `Enter` or `Space`
- clicking it with the mouse
- pressing `Ctrl+A`

## Campus network detection

Before it attempts login, the program checks whether at least one configured campus gateway is reachable.

- `gateway_hosts` is the main signal and should include your wired/wireless campus gateways
- `ipv4_cidrs` is optional; when filled, it only adds context and does not block login by itself

With the current defaults, the daemon stays idle off-campus and only tries to authenticate when a campus gateway responds.

## Autostart

Install autostart for the current user:

```bash
campus-network-autologin autostart install
```

This writes a platform-specific startup file that points to the current executable absolute path:

- Windows: Startup folder `campus-network-autologin.vbs` (hidden window mode)
- Linux: `~/.config/systemd/user/campus-network-autologin.service`
- macOS: `~/Library/LaunchAgents/com.campus-network-autologin.plist`

Remove it:

```bash
campus-network-autologin autostart remove
```

## Scope

This version only covers:

- automatic login
- captive-portal detection
- connectivity verification

It does not implement self-service account queries, logout, or password management.
