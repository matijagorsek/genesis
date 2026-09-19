const $ = (id) => document.getElementById(id);
const api = async (path, body) => {
  const r = await fetch(path, body ? { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify(body) } : {});
  return { ok: r.ok, data: await r.json().catch(() => ({})) };
};
const say = (where, text, kind = "bad") => { $(where).innerHTML = text ? `<div class="msg ${kind}">${text}</div>` : ""; };

// ---- services -------------------------------------------------------------
async function loadServices() {
  const { data } = await api("/api/services");
  $("no-firefox").hidden = !!data.firefox;
  $("tiles").innerHTML = "";
  for (const s of data.services || []) {
    const b = document.createElement("button");
    b.className = "tile";
    b.innerHTML = `<div class="dot" style="background:${s.colour}"></div><b></b><small></small>`;
    b.querySelector("b").textContent = s.name;
    b.querySelector("small").textContent = s.drm ? "needs Widevine" : "plays without DRM";
    b.onclick = async () => {
      b.querySelector("small").textContent = "opening…";
      const { data } = await api("/api/open", { id: s.id });
      b.querySelector("small").textContent = s.drm ? "needs Widevine" : "plays without DRM";
      say("service-msg", data.error || "", data.error ? "bad" : "ok");
    };
    $("tiles").appendChild(b);
  }
}

// ---- iptv -----------------------------------------------------------------
let channels = [];

function renderChannels(filter = "") {
  const f = filter.trim().toLowerCase();
  const shown = f ? channels.filter((c) => c.name.toLowerCase().includes(f)) : channels;
  const box = $("channels");
  box.innerHTML = "";
  for (const c of shown.slice(0, 600)) {
    const row = document.createElement("div");
    row.className = "chan";
    row.innerHTML = `<img alt="" loading="lazy"><span></span><em></em>`;
    if (c.logo) row.querySelector("img").src = c.logo;
    row.querySelector("span").textContent = c.name;
    row.querySelector("em").textContent = "play";
    row.onclick = async () => {
      const { data } = await api("/api/play", { url: c.url });
      say("iptv-msg", data.error ? data.error : `Playing in ${data.player}.`, data.error ? "bad" : "ok");
    };
    box.appendChild(row);
  }
  if (!shown.length) box.innerHTML = `<p class="note">Nothing matches that.</p>`;
  else if (shown.length > 600) box.insertAdjacentHTML("beforeend", `<p class="note">Showing the first 600 of ${shown.length}. Search to narrow it.</p>`);
}

async function loadChannels() {
  say("iptv-msg", "Asking your provider what this subscription carries…", "ok");
  const { data } = await api("/api/iptv/channels");
  if (data.error) { say("iptv-msg", data.error); return; }
  channels = data.channels || [];
  say("iptv-msg", `${data.total} channels.`, "ok");
  $("search").hidden = false;
  renderChannels($("search").value);
}

async function loadIptvState() {
  const { data } = await api("/api/iptv");
  $("forget").hidden = !data.configured;
  if (data.configured) {
    $("kind").value = data.kind || "xtream";
    $("host").value = data.host || "";
    $("username").value = data.username || "";
    onKind();
    loadChannels();
  }
}

function onKind() {
  const m3u = $("kind").value === "m3u";
  $("m3u-field").hidden = !m3u;
  $("xtream-fields").hidden = m3u;
}

// ---- wiring ---------------------------------------------------------------
$("tab-services").onclick = () => { $("services").hidden = false; $("iptv").hidden = true; $("tab-services").ariaSelected = "true"; $("tab-iptv").ariaSelected = "false"; };
$("tab-iptv").onclick = () => { $("services").hidden = true; $("iptv").hidden = false; $("tab-services").ariaSelected = "false"; $("tab-iptv").ariaSelected = "true"; };
$("kind").onchange = onKind;
$("search").oninput = (e) => renderChannels(e.target.value);

$("iptv-form").onsubmit = async (e) => {
  e.preventDefault();
  const body = { kind: $("kind").value, host: $("host").value, username: $("username").value, password: $("password").value, m3u: $("m3u").value };
  const { data } = await api("/api/iptv", body);
  if (data.error) { say("iptv-msg", data.error); return; }
  $("forget").hidden = false;
  loadChannels();
};

$("forget").onclick = async () => {
  await api("/api/iptv/forget", {});
  channels = []; $("channels").innerHTML = ""; $("search").hidden = true; $("forget").hidden = true;
  $("host").value = $("username").value = $("password").value = $("m3u").value = "";
  say("iptv-msg", "Forgotten. Nothing of your provider is kept on this machine.", "ok");
};

onKind();
loadServices();
loadIptvState();
