#!/usr/bin/env python3
"""Smoke-test the built Linux window over WebKitWebDriver (standard library only).

Run on a graphical session, or under `dbus-run-session -- mutter --headless
--wayland --no-x11 --virtual-monitor 1440x1000 -- python3 ...`.
All profile data is isolated in a temporary home. nmcli is replaced by a
recording stub, so exercising Apply cannot modify a host connection.
"""
import json
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


def smoke(home):
    bin_dir = home / "bin"
    bin_dir.mkdir()
    stub = bin_dir / "nmcli"
    stub.write_text('#!/bin/sh\nprintf "%s\\n" "$*" >> "$HOME/nmcli.log"\n'
                    'case "$*" in *"NAME,DEVICE"*) echo "Studio:test0";; esac\n')
    stub.chmod(0o755)
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

        def js(script, *args):
            return api(session + "/execute/sync", {"script": script, "args": list(args)})

        def wait(condition):
            until(lambda: js("return !!(" + condition + ")"))

        def click(selector):
            js("const b=document.querySelector(arguments[0]); if(!b || b.disabled) throw Error(arguments[0]); b.click()", selector)

        def toggle_sidebar():
            element = api(session + "/element", {"using":"css selector", "value":"#sidebar-toggle"})
            api(session + "/element/" + next(iter(element.values())) + "/click", {})

        wait("document.querySelector('#link-type') && !document.querySelector('#link-type').textContent.includes('Reading')")
        assert not js("return document.querySelector('#error').textContent")
        if not js("return document.querySelector('#pause-traffic').disabled"):
            click('#pause-traffic')
            wait("document.querySelector('#sample-state').textContent === 'Paused'")
            click('#pause-traffic')
            wait("document.querySelector('#pause-traffic').dataset.paused === 'false'")
        click('[data-do="new"]')
        js("document.querySelector('#p-name').value = arguments[0]", "Office <test>")
        # Exercise backend validation without depending on host adapter names.
        js("""const s=document.querySelector('#p-interface'); s.add(new Option('test0','test0')); s.value='test0';
              document.querySelector('#p-method').value='auto'; document.querySelector('#p-dns').value='invalid';
              document.querySelector('#editor').requestSubmit()""")
        wait("document.querySelector('#dialog-error')?.textContent.includes('Invalid DNS')")
        js("document.querySelector('#p-dns').value='1.1.1.1'; document.querySelector('#editor').requestSubmit()")
        wait("!document.querySelector('dialog').open && document.querySelector('[data-profile]')")
        click('[data-profile]')
        click('[data-do="edit"]')
        js("document.querySelector('#p-description').value='Edited configuration'; document.querySelector('#editor').requestSubmit()")
        wait("!document.querySelector('dialog').open && document.querySelector('.profile-text').textContent.includes('Edited')")
        click('[data-profile]')
        click('[data-do="apply"]')
        click('[data-do="confirm-apply"]')
        wait("!document.querySelector('dialog').open && document.querySelector('.profile-list .latest')?.textContent === 'Last applied'")
        assert "connection modify Studio ipv4.dns 1.1.1.1" in (home / "nmcli.log").read_text()
        cache = home / ".local/share/daggas/network-manager/profiles_cache.json"
        saved, = json.loads(cache.read_text())
        assert saved["metadata"]["name"] == "Office <test>" and saved["metadata"]["last_applied_at"]
        click('nav [data-page="activity"]')
        wait("document.querySelector('.logs').textContent.includes('Applied profile: Office <test>')")
        click('nav [data-page="settings"]')
        js("const t=document.querySelector('#theme'); t.value='dark'; t.dispatchEvent(new Event('change',{bubbles:true}))")
        assert js("return document.documentElement.dataset.theme") == "dark"
        click('nav [data-page="interfaces"]')
        api(session + "/window/rect", {"width": 550, "height": 550})
        click('nav [data-page="overview"]')
        wait("document.documentElement.dataset.sidebar === 'compact'")
        toggle_sidebar()
        wait("document.querySelector('#sidebar-toggle').getAttribute('aria-expanded') === 'true'")
        assert js("return document.querySelector('main').scrollWidth <= document.querySelector('main').clientWidth")
        assert js("""return document.querySelector('.connection').getBoundingClientRect().top >=
            document.querySelector('.traffic').getBoundingClientRect().bottom""")
        assert js("return document.querySelector('nav [aria-current=\"page\"]').dataset.page") == 'overview'
        assert js("""const main=document.querySelector('main'); main.scrollTop=main.scrollHeight;
            const side=document.querySelector('.sidebar').getBoundingClientRect();
            const b=document.querySelector('#sidebar-toggle').getBoundingClientRect();
            return main.scrollTop>0 && side.top===0 && Math.abs(side.height-innerHeight)<1 &&
                document.elementFromPoint(b.x+b.width/2,b.y+b.height/2).closest('#sidebar-toggle')!==null""")
        toggle_sidebar()
        wait("document.querySelector('#sidebar-toggle').getAttribute('aria-label') === 'Expand sidebar'")
        api(session + "/window/rect", {"width": 1220, "height": 820})
        api(session + "/refresh", {})
        wait("document.querySelector('#sidebar-toggle')?.getAttribute('aria-expanded') === 'false'")
        toggle_sidebar()
        wait("document.documentElement.dataset.sidebar === 'expanded' && document.querySelector('[data-profile]')")
        def shortcut():
            api(session + "/actions", {"actions":[{"type":"key","id":"keyboard","actions":[
                {"type":"keyDown","value":"\ue009"}, {"type":"keyDown","value":"b"},
                {"type":"keyUp","value":"b"}, {"type":"keyUp","value":"\ue009"}]}]})
        shortcut()
        wait("document.querySelector('#sidebar-toggle').getAttribute('aria-expanded') === 'false'")
        shortcut()
        wait("document.querySelector('#sidebar-toggle').getAttribute('aria-expanded') === 'true'")
        click('#commands')
        click('.command-list [data-page="profiles"]')
        click('[data-profile]')
        click('[data-do="remove"]')
        click('[data-do="confirm-remove"]')
        wait("!document.querySelector('dialog').open && !document.querySelector('[data-profile]')")
        assert json.loads(cache.read_text()) == []
        print("Native workspace: profile workflows, themes, chart controls, full-height sidebar, collapse/expand, saved state and compact layout passed.")
    finally:
        try:
            if session:
                api(session, method="DELETE")
        finally:
            import signal
            os.killpg(driver.pid, signal.SIGTERM)
            driver.wait(timeout=5)


if __name__ == "__main__":
    with tempfile.TemporaryDirectory(prefix="netmanager-ui-") as tmp:
        smoke(Path(tmp))
