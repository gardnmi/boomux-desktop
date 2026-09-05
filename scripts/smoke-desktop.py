"""Run a release bundle against an isolated X11 or Wayland display and daemon."""

import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import signal
import subprocess
import tarfile
import tempfile
import time


def wait_for(description, predicate, processes, seconds=30):
    deadline = time.monotonic() + seconds
    while time.monotonic() < deadline:
        for process in processes:
            if process.poll() is not None:
                raise RuntimeError(f"{description}: process exited with {process.returncode}")
        if predicate():
            return
        time.sleep(0.2)
    raise RuntimeError(f"timed out waiting for {description}")


def wayland_frame_presented(log):
    if 'set_app_id("org.omarchy.boomux-desktop")' not in log:
        return False
    # Follow a toplevel's surface, rather than accepting a cursor buffer or an
    # initial registry sync as evidence that the app submitted a window frame.
    for surface in re.findall(r"get_xdg_surface\([^\n]*wl_surface[@#](\d+)", log):
        attached = re.search(rf"wl_surface[@#]{surface}\.attach\(wl_buffer[@#]\d+", log)
        if attached:
            committed = re.search(rf"wl_surface[@#]{surface}\.commit\(\)", log[attached.end():])
            if committed and re.search(r"wl_callback[@#]\d+\.done\(",
                                       log[attached.end() + committed.end():]):
                return True
    return False


def created_id(output):
    identities = re.findall(r"\(([0-9a-f]{8}(?:-[0-9a-f]{4}){3}-[0-9a-f]{12})\)", output)
    if len(identities) != 1:
        raise RuntimeError(f"expected one exact resource ID in CLI response: {output}")
    return identities[0]


def stop(process):
    if process.poll() is None:
        process.terminate()
        try:
            process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            process.kill()
            process.wait(timeout=5)


def smoke(backend, archive, output, software_driver=None):
    output.mkdir(parents=True, exist_ok=True)
    expected = Path(str(archive) + ".sha256").read_text().split()[0]
    with archive.open("rb") as source:
        actual = hashlib.file_digest(source, "sha256").hexdigest()
    if actual != expected:
        raise RuntimeError("release archive checksum mismatch")

    with tempfile.TemporaryDirectory(prefix="boomux-smoke-") as directory:
        root = Path(directory)
        bundle = root / "bundle"
        bundle.mkdir()
        with tarfile.open(archive) as tar:
            tar.extractall(bundle, filter="data")
        env = {key: value for key, value in os.environ.items()
               if not key.startswith(("BOOMUX_", "HYPRLAND_", "VK_", "ZED_"))
               and key not in {"DISPLAY", "WAYLAND_DISPLAY", "WAYLAND_SOCKET", "WAYLAND_DEBUG",
                               "DBUS_SESSION_BUS_ADDRESS", "DBUS_SESSION_BUS_PID", "XAUTHORITY",
                               "SESSION_MANAGER", "DESKTOP_SESSION", "XDG_CURRENT_DESKTOP"}}
        for name in ["RUNTIME_DIR", "CONFIG_HOME", "STATE_HOME", "DATA_HOME", "CACHE_HOME"]:
            path = root / name.lower()
            path.mkdir(mode=0o700)
            env[f"XDG_{name}"] = str(path)
        env.update(LIBGL_ALWAYS_SOFTWARE="1", GALLIUM_DRIVER="llvmpipe",
                   XDG_SESSION_TYPE=backend, SHELL="/bin/sh", RUST_BACKTRACE="1")
        drivers = ([software_driver.resolve()] if software_driver else
                   sorted(Path("/usr/share/vulkan/icd.d").glob("lvp*.json")))
        if not drivers:
            raise RuntimeError("Mesa lavapipe is required (install mesa-vulkan-drivers)")
        env["VK_DRIVER_FILES"] = str(drivers[0])
        env["VK_ICD_FILENAMES"] = str(drivers[0])
        processes, apps, logs = [], [], []
        shell_id = None

        def start(command, name, child_env=None, **kwargs):
            log = (output / f"{name}.log").open("wb")
            logs.append(log)
            process = subprocess.Popen(command, env=child_env or env, cwd=root,
                                       stdout=log, stderr=subprocess.STDOUT, **kwargs)
            processes.append(process)
            return process

        def cli(*args, check=True):
            result = subprocess.run([bundle / "bin/boomux", *args], env=env, cwd=root,
                                    capture_output=True, text=True, timeout=10)
            with (output / "boomux.log").open("a") as log:
                log.write(f"{args!r}\n{result.stdout}{result.stderr}\n")
            if check and result.returncode:
                raise RuntimeError(f"Boomux command failed: {args}: {result.stderr}")
            return result.stdout

        def inspect():
            return json.loads(cli("--json", "shell", "inspect", shell_id))["data"]["shell"]

        try:
            # An owned bus with no activation directories cannot launch the
            # host's portal/desktop services using an inherited session.
            bus_config = root / "dbus.conf"
            bus_config.write_text(
                '<busconfig><type>session</type>'
                f'<listen>unix:tmpdir={env["XDG_RUNTIME_DIR"]}</listen>'
                '<policy context="default"><allow send_destination="*"/>'
                '<allow receive_sender="*"/><allow own="*"/></policy></busconfig>')
            bus_address = root / "bus-address"
            with bus_address.open("wb") as descriptor:
                bus = start(["dbus-daemon", "--nofork", f"--config-file={bus_config}",
                             f"--print-address={descriptor.fileno()}"], "dbus",
                            pass_fds=(descriptor.fileno(),))
            wait_for("private D-Bus", lambda: bus_address.read_text().strip(), [bus])
            env["DBUS_SESSION_BUS_ADDRESS"] = bus_address.read_text().strip()
            # Xvfb chooses a free display number. Weston uses its X11 backend
            # to provide the input seat GPUI requires, still without hardware.
            display_file = root / "display"
            with display_file.open("wb") as descriptor:
                display = start(["Xvfb", "-displayfd", str(descriptor.fileno()), "-screen", "0",
                                 "1280x800x24", "-nolisten", "tcp", "-ac"], "xvfb",
                                pass_fds=(descriptor.fileno(),))
            wait_for("Xvfb readiness", lambda: display_file.read_text().strip(), [display])
            env["DISPLAY"] = ":" + display_file.read_text().strip()
            servers = [bus, display]
            if backend == "wayland":
                weston = start(["weston", "--backend=x11", "--renderer=pixman",
                                "--shell=kiosk-shell.so",
                                "--socket=wayland-smoke", "--no-config", "--idle-time=0",
                                "--width=1280", "--height=800"], "weston")
                servers.append(weston)
                socket = Path(env["XDG_RUNTIME_DIR"]) / "wayland-smoke"
                wait_for("Weston readiness", socket.is_socket, servers)
                env["WAYLAND_DISPLAY"] = "wayland-smoke"
                # No X11 fallback is possible for the application.
                del env["DISPLAY"]

            def launch(name):
                child_env = dict(env)
                if backend == "wayland":
                    child_env["WAYLAND_DEBUG"] = "client"
                app = start([bundle / "bin/boomux-desktop"], name, child_env)
                apps.append(app)
                log_path = output / f"{name}.log"

                def visible():
                    if backend == "wayland":
                        return wayland_frame_presented(log_path.read_text(errors="replace"))
                    tree = subprocess.run(["xwininfo", "-root", "-tree"], env=env,
                                          capture_output=True, text=True, timeout=5).stdout
                    (output / f"{name}-windows.txt").write_text(tree)
                    for window in re.findall(r'(0x[0-9a-f]+) "[^"\n]*Boomux Desktop[^"\n]*"', tree):
                        details = subprocess.run(["xwininfo", "-id", window], env=env,
                                                 capture_output=True, text=True, timeout=5).stdout
                        if "Map State: IsViewable" in details:
                            return True
                    return False

                wait_for(f"{backend} window/frame", visible, [*servers, app])
                return app

            # The bundled launcher must start a daemon on a clean runtime.
            app = launch("empty-start")
            status = json.loads(cli("--json", "daemon", "status"))["data"]
            if status["status"] != "running":
                raise RuntimeError(f"launcher did not start Boomux: {status}")
            if status["socket_path"] != str(Path(env["XDG_RUNTIME_DIR"]) / "boomux/daemon.sock"):
                raise RuntimeError("daemon did not use the isolated runtime")
            daemon_pid = status["pid"]
            if daemon_pid is None:
                raise RuntimeError("could not identify the isolated daemon process")
            stop(app)
            workspace_id = created_id(cli("workspace", "create", "desktop-smoke"))
            shell_id = created_id(cli("shell", "create", workspace_id, "--name", "smoke-shell",
                                      "--cwd", str(root), "--", "/bin/sh", "-c",
                                      "printf 'boomux-desktop-smoke-ready\\n'; exec sleep 180"))
            if inspect()["status"] != "pending":
                raise RuntimeError("fixture Shell must be pending before Desktop attaches")
            env["BOOMUX_DESKTOP_SHELL_ID"] = shell_id
            app = launch("shell-attach")
            wait_for("Desktop attachment and PTY output",
                     lambda: inspect()["status"] == "running" and
                     (inspect().get("run") or {}).get("output_revision", 0) > 0,
                     [*servers, app])
            run_id = inspect()["run"]["id"]
            # Keep the real application active long enough to catch startup failures.
            deadline = time.monotonic() + 3
            wait_for("startup settling", lambda: time.monotonic() >= deadline,
                     [*servers, app], seconds=5)
            stop(app)
            after = inspect()
            if after["status"] != "running" or after["run"]["id"] != run_id:
                raise RuntimeError("exiting Desktop did not preserve the exact ShellRun")
            app = launch("shell-reattach")
            after = inspect()
            if after["status"] != "running" or after["run"]["id"] != run_id:
                raise RuntimeError("reopening Desktop changed the ShellRun")
            if json.loads(cli("--json", "daemon", "status"))["data"]["pid"] != daemon_pid:
                raise RuntimeError("reopening Desktop replaced the daemon")
            (output / "result.json").write_text(json.dumps(
                dict(backend=backend, shell_id=shell_id, run_id=run_id, status="passed",
                     archive_sha256=actual, boomux_version=cli("--version").strip()), indent=2))
            print(f"PASS: {backend} bundle startup, attachment, and ShellRun survival", flush=True)
        finally:
            # Stop clients first, then only the resources in this private runtime.
            for process in reversed(apps):
                stop(process)
            try:
                if shell_id:
                    cli("shell", "close", shell_id, check=False)
                cli("daemon", "stop", check=False)
            finally:
                for process in reversed(processes):
                    stop(process)
                for log in logs:
                    log.close()


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--backend", choices=["x11", "wayland"], required=True)
    parser.add_argument("--archive", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--software-driver", type=Path,
                        help="lavapipe ICD JSON for a non-system Mesa installation")
    args = parser.parse_args()

    def interrupted(signum, _frame):
        raise RuntimeError(f"smoke test interrupted by signal {signum}")

    signal.signal(signal.SIGTERM, interrupted)
    smoke(args.backend, args.archive.resolve(), args.output.resolve(), args.software_driver)
