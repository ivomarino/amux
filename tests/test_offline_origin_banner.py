"""AC-294: a proxied origin must not be told to install Tailscale or run mkcert.

The defect this guards: /api/offline-origin introspected the server's OWN certificate
and the client rendered the result as the reason a service worker failed. Behind a
reverse proxy those are different facts — the gateway terminates TLS with a real cert,
and the container cannot see that — so every cloud workspace showed a full-width red
banner reading "No Tailscale hostname found, so amux only has a self-signed cert...
Install Tailscale, or run `mkcert` and restart amux". Confirmed in two prospect orgs by
looking at the rendered page; the API layer was green the whole time.

These are source-shape tests, which is the honest limit of what a unit test reaches for
a handler wired into a 75k-line single file. They are written to fail if the fix is
reverted, and each was confirmed red against the pre-fix source before being committed.
"""
import re
from pathlib import Path

SRC = (Path(__file__).parent.parent / "amux-server.py").read_text()


def _handler(code_only=False):
    """The /api/offline-origin handler body, up to the next route.

    code_only=True strips whole-line `#` comments first. This is load-bearing, not
    tidiness: the fix's own comment QUOTES the banner text it removed ("Install
    Tailscale, or run `mkcert`"), so a positional test that searches raw source finds
    the explanation instead of the code and reports the ordering backwards. That is
    exactly what happened on the first run of this file — the test failed against the
    CORRECT fix. A probe confounded by prose that looks like its target.
    """
    m = re.search(r'if method == "GET" and path == "/api/offline-origin":', SRC)
    assert m, "the /api/offline-origin route is gone"
    rest = SRC[m.start():]
    nxt = re.search(r'\n        # ── ', rest)
    body = rest[:nxt.start() if nxt else 4000]
    if code_only:
        body = "\n".join(l for l in body.splitlines() if not l.lstrip().startswith("#"))
    return body


def _client_fn():
    m = re.search(r"async function _swOfferGoodOrigin\(\)", SRC)
    assert m, "_swOfferGoodOrigin is gone"
    rest = SRC[m.start():]
    end = re.search(r"\n\}\n", rest)
    return rest[:end.start() if end else 3000]


def test_proxied_request_is_detected_at_all():
    """The gateway-injected header is the only thing that can distinguish a hosted
    request from a local one without an IS_CLOUD branch (single-codebase rule)."""
    h = _handler()
    assert "X-Forwarded-Proto" in h, (
        "the handler no longer looks at X-Forwarded-Proto, so it is back to reporting "
        "its own cert as the browser's — that is AC-294")


def test_proxied_branch_returns_BEFORE_the_tailscale_advice():
    """ORDER IS THE FIX. The mkcert/Tailscale text must be unreachable for a proxied
    request, not merely followed by something friendlier."""
    h = _handler(code_only=True)
    fwd = h.index("X-Forwarded-Proto")
    advice = h.find("Install Tailscale")
    assert advice != -1, "the local advice is gone — it is CORRECT for a local OSS user"
    assert fwd < advice, (
        "the Tailscale/mkcert advice is reachable before the proxied check — a hosted "
        "user can still be told to install a VPN for a machine they do not have")
    seg = h[fwd:advice]
    assert "return self._json" in seg, (
        "the proxied branch does not RETURN, so control falls through to the cert story")


def test_the_proxied_response_carries_no_cert_advice():
    """Scope is the PROXIED branch only — from the header check to where the
    non-proxied response begins.

    My first version sliced to "Install Tailscale" instead, which swallowed the entire
    local response, and that response legitimately contains "self-signed" in its FIRST
    ternary arm. So the test failed against the correct fix for the second time in one
    sitting: same root as the comment-quoting confound above, one layer over. A
    positional probe over a 75k-line single file will keep finding neighbours unless its
    end boundary is as deliberate as its start.
    """
    h = _handler(code_only=True)
    fwd = h.index("X-Forwarded-Proto")
    first_ret = h.index("return self._json", fwd)
    local_ret = h.index("return self._json", first_ret + len("return self._json"))
    branch = h[fwd:local_ret]
    for bad in ("mkcert", "Install Tailscale", "self-signed"):
        assert bad not in branch, (
            "the proxied response still mentions %r — the whole finding was that cert "
            "advice is unactionable for a hosted user" % bad)


def test_client_suppresses_the_red_bar_on_a_proxied_origin():
    fn = _client_fn()
    assert "info.proxied" in fn, (
        "the client no longer checks the proxied flag, so the red bar renders on cloud "
        "again even though the server now reports the truth")
    guard = fn.index("info.proxied")
    made = fn.find("createElement('div')")
    assert made == -1 or guard < made, (
        "the bar element is built before the proxied guard — de-escalation has to happen "
        "before the DOM node exists, or a later `return` still leaves it appended")


def test_the_failure_is_de_escalated_NOT_silenced():
    """A suppressed banner must not become an undiagnosable failure (ethos rule 4).
    console.warn at the call site, localStorage and _swFailure all still record it."""
    fn = _client_fn()
    assert "console.warn" in fn, (
        "the proxied path neither shows nor logs anything — that is a silent failure, "
        "which is a different bug rather than a fix for this one")
    assert "amux_sw_error" in SRC and "_swFailure" in SRC, (
        "the durable record of the real SW error is gone")


def test_client_and_server_agree_on_the_FIELD_NAME():
    """Two components describing the same fact must not drift. The server emitting
    `proxied` while the client reads something else is a fix that reaches nobody, and it
    would pass every other test in this file."""
    assert '"proxied": True' in _handler(), "server no longer emits the `proxied` field"
    assert "info.proxied" in _client_fn(), "client no longer reads the `proxied` field"
