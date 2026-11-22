#!/usr/bin/env python3
"""Smoke-test the built Linux window over WebKitWebDriver (standard library only).

Run on a graphical session, or under `dbus-run-session -- mutter --headless
--wayland --no-x11 --virtual-monitor 1440x1000 -- python3 ...`.
All profile data is isolated in a temporary home. nmcli is replaced by a
recording stub, so exercising Apply cannot modify a host connection.
"""
import json
import base64
import os
from pathlib import Path
import socket
import subprocess
import tempfile
import time
from urllib.error import HTTPError, URLError
from urllib.request import Request, urlopen

ROOT = Path(__file__).resolve().parents[1]
APP = ROOT / "src-tauri/target/debug/network-manager-workspace"


def until(check, seconds=12):
    end = time.monotonic() + seconds
    while time.monotonic() < end:
        result = check()
        if result:
            return result
        time.sleep(0.08)
    raise AssertionError(f"Condition did not become true: {check}")


def smoke(home, previews=None):
    bin_dir = home / "bin"
    bin_dir.mkdir()
    stub = bin_dir / "nmcli"
    stub.write_text('#!/bin/sh\nprintf "%s\\n" "$*" >> "$HOME/nmcli.log"\n'
                    'case "$*" in *"NAME,DEVICE"*) echo "Studio:eth0"; for d in /sys/class/net/*; do echo "Studio:${d##*/}"; done;; *"GENERAL.CONNECTION"*) echo Studio;; *"STATE general"*) echo connected;; esac\n')
    stub.chmod(0o755)
    for name, script in {"systemctl": "exit 0", "notify-send": "exit 0",
                         "ping": "echo '64 bytes from localhost: time=0.1 ms'","xdg-open":"exit 0"}.items():
        path = bin_dir / name
        path.write_text("#!/bin/sh\n"+script+"\n")
        path.chmod(0o755)
    store = home / ".local/share/daggas/network-manager"
    store.mkdir(parents=True)
    (store / "settings.toml").write_text("show_tray_icon = false\nshow_notifications = false\nauto_switch_enabled = false\n")
    env = dict(os.environ, HOME=str(home), XDG_DATA_HOME=str(home / "data"),
               XDG_CONFIG_HOME=str(home / "config"), XDG_CACHE_HOME=str(home / "cache"),
               PATH=f"{bin_dir}:{os.environ['PATH']}", TAURI_WEBVIEW_AUTOMATION="true")
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        port = s.getsockname()[1]
    base = f"http://127.0.0.1:{port}"

    def api(path, body=None, method=None):
        request = Request(base + path, method=method, headers={"Content-Type": "application/json"},
                          data=None if body is None else json.dumps(body).encode())
        try:
            with urlopen(request, timeout=25) as resp:
                return json.load(resp)["value"]
        except HTTPError as e:
            raise RuntimeError(e.read().decode()) from e

    def ready():
        try:
            return api("/status")["ready"]
        except URLError:
            return False

    # Application single-instance registration must not find a user's
    # already-running workspace, even when this is run on their desktop.
    driver = subprocess.Popen(["dbus-run-session", "--", "WebKitWebDriver", "-p", str(port)], env=env,
                              start_new_session=True)
    session = None
    try:
        until(ready)
        capabilities = {"webkitgtk:browserOptions": {"binary": str(APP)}}
        session = "/session/" + api("/session", {"capabilities": {"alwaysMatch": capabilities}})["sessionId"]
        main_handle=api(session+"/window")
        active_window=main_handle

        def js(script, *args):
            return api(session + "/execute/sync", {"script": script, "args": list(args)})

        def wait(condition):
            try:
                until(lambda: js("return !!(" + condition + ")"))
            except AssertionError as err:
                raise AssertionError((condition,js("return [document.body.dataset.page,document.querySelector('#error')?.textContent,document.querySelector('#dialog-error')?.textContent]"))) from err

        def click(selector):
            wait("!document.body.hasAttribute('aria-busy')")
            js("const b=document.querySelector(arguments[0]); if(!b || b.disabled) throw Error(arguments[0]); b.click()", selector)

        def toggle_sidebar():
            element = api(session + "/element", {"using":"css selector", "value":"#sidebar-toggle"})
            api(session + "/element/" + next(iter(element.values())) + "/click", {})

        def val(selector, value, event="input"):
            js("const n=document.querySelector(arguments[0]);n.value=arguments[1];n.dispatchEvent(new Event(arguments[2],{bubbles:true}))",selector,value,event)
        def submit(form):
            js("document.querySelector(arguments[0]).requestSubmit()", form)
        def nav(page):
            click(f'nav [data-page="{page}"]')
            wait(f"document.body.dataset.page==='{page}'")
        def act(action):
            nonlocal active_window
            handles=api(session+"/window/handles")
            control = "window-close" if action=="close" and active_window!=main_handle else action
            click(f'[data-do="{control}"]')
            if action in ["new","edit","switch","preferences","diagnostics","schedules","hotkeys","rules","scan-details"]:
                active_window=until(lambda:next((w for w in api(session+"/window/handles") if w not in handles),None))
                api(session+"/window",{"handle":active_window})
                wait("document.querySelector('dialog[open]') && !document.body.hasAttribute('aria-busy')")
                assert js("""const h=document.querySelector('.content-header');
                    return h.getBoundingClientRect().top===0 && h.offsetHeight>0 &&
                        document.querySelector('#heading').textContent===document.title &&
                        document.querySelector('[data-do="window-close"]').offsetWidth>0""")
            elif action=="close" and active_window!=main_handle:
                closed()
        def closed():
            nonlocal active_window
            if active_window!=main_handle:
                until(lambda:active_window not in api(session+"/window/handles"))
                active_window=main_handle
                api(session+"/window",{"handle":main_handle})
            wait("!document.querySelector('dialog').open")
        def capture(name):
            if previews:
                js("document.querySelector('#notice').textContent='';document.activeElement.blur()")
                (previews / (name+".png")).write_bytes(base64.b64decode(api(session+"/screenshot")))
        wait("document.querySelector('.service-state strong')?.textContent==='Running'")
        assert js("return [...document.querySelectorAll('nav button')].map(n=>n.textContent)") == [
            "Dashboard","Profiles","Scanner","Port Scanner","Logs","Settings","Help"]
        act("pause-traffic")
        wait("document.querySelector('#sample-state').textContent==='Paused'")
        act("pause-traffic")
        act("switch");act("close")
        act("new")
        wait("document.querySelector('#profile-editor')")
        val("#template","1","change")
        wait("document.querySelector('#p-name').value==='Home Network'")
        val("#p-name","Office <test>")
        submit("#profile-editor")
        closed()
        nav("profiles")
        act("edit")
        wait("document.querySelector('#profile-editor')")
        val("#p-description","Edited configuration")
        val("#p-group","Work")
        val("#adapter-0-dns1","8.8.4.4")
        val("#adapter-0-dns2","9.9.9.9")
        submit("#profile-editor")
        closed()
        act("apply")
        wait("document.querySelector('[data-do=\"confirm-apply\"]')")
        act("confirm-apply")
        closed()
        cache=store/"profiles_cache.json"
        saved,=json.loads(cache.read_text())
        assert saved["metadata"]["last_applied_at"] and saved["metadata"]["group"]["name"]=="Work"
        assert "connection modify Studio ipv4.dns 8.8.4.4,9.9.9.9" in (home / "nmcli.log").read_text()
        act("duplicate")
        wait("document.querySelectorAll('.profile-rows li').length===2")
        act("remove");act("confirm-remove");closed()
        js("""const f=document.querySelector('#import-file'),dt=new DataTransfer();
          dt.items.add(new File([arguments[0]],'backup.json',{type:'application/json'}));f.files=dt.files;f.dispatchEvent(new Event('change',{bubbles:true}));""",
           json.dumps({"profiles":[dict(saved,auto_switch_rules={"conditions":[{"type":"network_available"}]},automation_actions=[{"action":"notification","title":"Imported","body":"Hello"}])]}))
        wait("document.querySelector('[data-do=\"import-safe\"]')")
        act("import-safe");closed()
        imported=next(p for p in json.loads(cache.read_text()) if "(Imported)" in p["metadata"]["name"])
        assert not imported.get("automation_actions") and not imported["approved_for_users"]
        nav("scanner")
        val("#scan-target","127.0.0.1")
        submit("#scan-form")
        wait("document.querySelector('#scan-status').textContent.startsWith('Scan complete')")
        act("scan-details")
        wait("document.querySelector('#heading')?.textContent==='Device Details'")
        act("scan-copy-field")
        act("close")
        nav("ports")
        with socket.socket() as listener:
            listener.bind(("127.0.0.1",0));listener.listen()
            port=listener.getsockname()[1]
            val("#scan-target","127.0.0.1");val("#scan-mode","custom","change");val("#scan-custom",str(port))
            submit("#scan-form")
            wait(f"document.querySelector('tr[data-device=\"{port}\"]')")
            js("""document.querySelector('[data-resize="port"]').dispatchEvent(
              new KeyboardEvent('keydown',{key:'ArrowRight',bubbles:true}))""")
            assert js("return Number(document.querySelector('col[data-column=\"port\"]').getAttribute('width'))") >= 64
        nav("logs")
        wait("document.querySelector('.log-text').textContent.includes('Applied profile: Office <test>')")
        val("#query","zz-no-such-event")
        wait("document.querySelector('.log-text').textContent.includes('No matching')")
        nav("settings")
        act("enable-encryption")
        val("#new-key","a test passphrase");val("#repeat-key","a test passphrase");submit("#encryption-form");closed()
        assert not cache.exists() and (store/"profiles_cache.json.enc").exists()
        act("change-encryption")
        val("#current-key","a test passphrase");val("#new-key","replacement passphrase");val("#repeat-key","replacement passphrase");submit("#encryption-form");closed()
        act("disable-encryption")
        val("#current-key","replacement passphrase");submit("#encryption-form");closed()
        assert cache.exists() and not (store/"profiles_cache.json.enc").exists()
        for kind in ["schedules","hotkeys"]:
            act(kind);act("entry-add")
            if kind=="hotkeys":val("#entry-key","F8")
            submit("#entry-form")
            wait("document.querySelector('[data-do=\"entry-delete\"]')")
            act("entry-delete")
            wait("!document.querySelector('[data-do=\"entry-delete\"]')")
            act("close")
        click("#main-menu");act("preferences");act("pref-behavior")
        wait("document.querySelector('#opt-confirm_profile_switch')")
        click("#opt-confirm_profile_switch")
        wait("!document.querySelector('#opt-confirm_profile_switch').checked")
        act("close")
        val("#opt-auto_switch_interval_secs","5","change")
        wait("!document.body.hasAttribute('aria-busy')")
        click("#opt-auto_switch_enabled")
        until(lambda:any(p["metadata"].get("last_applied_at") for p in json.loads(cache.read_text()) if "(Imported)" in p["metadata"]["name"]))
        click("#opt-auto_switch_enabled")
        nav("help")
        act("shortcuts");act("close")
        click("#main-menu");act("about");wait("document.querySelector('#dialog-title').textContent.includes('About')");act("close")
        api(session+"/window/rect",{"width":550,"height":500})
        wait("document.documentElement.dataset.sidebar==='compact'")
        toggle_sidebar()
        assert js("""const main=document.querySelector('main');main.scrollTop=main.scrollHeight;
           const r=document.querySelector('.sidebar').getBoundingClientRect();
           return main.scrollTop>0 && main.scrollWidth<=main.clientWidth && r.top===0 && Math.abs(r.height-innerHeight)<1""")
        for page in ["dashboard","profiles","scanner","ports","logs","settings","help"]:
            nav(page)
            assert js("""const m=document.querySelector('main');
                return m.scrollWidth<=m.clientWidth && document.querySelector('#page-title').offsetHeight>0"""), page
        toggle_sidebar()
        api(session + "/window/rect", {"width": 1152, "height": 680})
        api(session + "/refresh", {})
        wait("document.querySelector('#sidebar-toggle')?.getAttribute('aria-expanded')==='false' && document.querySelector('#version')?.textContent")
        toggle_sidebar()
        def shortcut():
            api(session + "/actions", {"actions":[{"type":"key","id":"keyboard","actions":[
                {"type":"keyDown","value":"\ue009"}, {"type":"keyDown","value":"b"},
                {"type":"keyUp","value":"b"}, {"type":"keyUp","value":"\ue009"}]}]})
        shortcut()
        wait("document.querySelector('#sidebar-toggle').getAttribute('aria-expanded') === 'false'")
        shortcut()
        wait("document.querySelector('#sidebar-toggle').getAttribute('aria-expanded') === 'true'")
        if previews:
            api(session+"/window/rect",{"width":1200,"height":760})
        # Keep an editor open across theme changes. Updating chrome must not
        # rebuild the form or discard unsaved input.
        nav("dashboard")
        act("new")
        editor=active_window
        val("#p-name","Unsaved draft")
        active_window=main_handle
        api(session+"/window",{"handle":main_handle})
        for theme in ["light","dark"]:
            nav("settings");val("#opt-theme",theme,"change")
            wait(f"document.documentElement.dataset.theme==='{theme}'")
            bar_color=js("return getComputedStyle(document.querySelector('.content-header')).backgroundColor")
            api(session+"/window",{"handle":editor})
            wait(f"document.documentElement.dataset.theme==='{theme}'")
            assert js("""return getComputedStyle(document.querySelector('.content-header')).backgroundColor
                ===arguments[0] && document.querySelector('#p-name').value==='Unsaved draft'""",bar_color)
            api(session+"/window",{"handle":main_handle})
            if previews:
                val("#interval","1000","change")
            nav("dashboard")
            if previews:
                wait("(document.querySelector('#in-line')?.getAttribute('d')||'').split('L').length>=6")
                js("""const cells=document.querySelectorAll('#network-facts dd');
                    ['192.0.2.24/24','192.0.2.1','192.0.2.53'].forEach((s,i)=>cells[i].textContent=s);
                    document.querySelector('#connection-heading').textContent='Studio';
                    const primary=document.querySelector('.connection-meta code');
                    const names=new Map([...document.querySelectorAll('.interface strong')].map((el,i)=>[el.textContent,'net'+i]));
                    if(primary)primary.textContent=names.get(primary.textContent)||'net0';
                    document.querySelectorAll('.interface strong').forEach(el=>el.textContent=names.get(el.textContent));
                    document.querySelector('.profile-current strong').textContent='Studio';
                    document.querySelector('.profile-current p').textContent='Applied: 09:41';""")
                capture(theme)
        if previews:
            toggle_sidebar();capture("compact")
        active_window=editor
        api(session+"/window",{"handle":editor})
        act("close")
        print("All seven native Tauri pages passed: profile CRUD/apply/import, both loopback scanners, logs, encryption rotation, schedules, hotkeys, preferences, sidebar and themes.")
    finally:
        try:
            if session:
                api(session, method="DELETE")
        finally:
            import signal
            os.killpg(driver.pid, signal.SIGTERM)
            driver.wait(timeout=5)


if __name__ == "__main__":
    import argparse
    parser=argparse.ArgumentParser()
    parser.add_argument("--previews",type=Path,help="Write anonymized native screenshots to this directory")
    args=parser.parse_args()
    if args.previews:args.previews.mkdir(parents=True,exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="netmanager-ui-") as tmp:
        smoke(Path(tmp),args.previews)
