// The embedded studio UI (kept in a separate file only to keep the
// server logic readable; included verbatim into studio.rs).
//
// Two tabs, by design: DESIGN (no code — sliders, texture drop zones,
// compose dialog, preview & actions) and CODE (the shader, for those
// who want it). The code tab is optional and always available.

const UI_HTML: &str = r#"<!doctype html>
<html lang=en>
<head>
<meta charset=utf-8>
<meta name=viewport content='width=device-width,initial-scale=1'>
<title>bruma studio</title>
<style>
:root { color-scheme: dark; }
* { box-sizing: border-box; }
body { margin: 0; font: 14px/1.5 system-ui, sans-serif; background: #0e0e12; color: #e8e8ec; }
header { display: flex; align-items: center; gap: 14px; padding: 10px 20px; border-bottom: 1px solid #22222c; flex-wrap: wrap; }
h1 { font-size: 16px; font-weight: 600; margin: 0; }
select, input { background: #1c1c26; color: #e8e8ec; border: 1px solid #33333d; border-radius: 8px; padding: 6px 10px; font: inherit; }
#tabs { display: flex; gap: 4px; margin-left: auto; }
.tab { padding: 7px 16px; border-radius: 8px; border: 1px solid transparent; background: none; color: #999; cursor: pointer; font: inherit; }
.tab.on { background: #1c1c26; border-color: #33333d; color: #fff; }
#hint { color: #888; font-size: 13px; width: 100%; }
main { padding: 18px 22px 90px; }
.panel { display: none; }
.panel.on { display: block; }
.cols { display: grid; grid-template-columns: 340px 1fr; gap: 22px; align-items: start; }
@media (max-width: 900px) { .cols { grid-template-columns: 1fr; } }
h2 { font-size: 12px; text-transform: uppercase; letter-spacing: .08em; color: #777; margin: 0 0 10px; }
.card { background: #16161e; border: 1px solid #232330; border-radius: 14px; padding: 16px; margin-bottom: 16px; }
.pgroup { margin-bottom: 16px; }
.pgroup label { display: flex; justify-content: space-between; font-size: 13px; margin-bottom: 5px; }
.pgroup .val { color: #7aa2f7; font-variant-numeric: tabular-nums; }
#layerList { list-style: none; margin: 0; padding: 0; }
.llayer { display: flex; align-items: center; gap: 6px; padding: 7px 8px; border: 1px solid #26262f; border-radius: 8px; margin-bottom: 6px; cursor: pointer; }
.llayer.on { border-color: #7aa2f7; background: #7aa2f711; }
.llayer .lname { flex: 1; font-size: 13px; }
.llayer .lmeta { color: #777; font-size: 11px; }
.llayer button { background: none; border: none; color: #888; cursor: pointer; font-size: 13px; padding: 0 3px; }
.llayer button:hover { color: #fff; }
.lrow { display: flex; gap: 8px; align-items: center; }
input[type=range] { width: 100%; accent-color: #7aa2f7; height: 28px; }
.tzone { border: 2px dashed #3a3a4a; border-radius: 12px; padding: 14px; text-align: center; color: #999; cursor: pointer; transition: all .15s; }
.tzone.over, .tzone:hover { border-color: #7aa2f7; color: #7aa2f7; background: #7aa2f711; }
.tzone img { max-width: 100%; max-height: 130px; border-radius: 8px; margin-bottom: 8px; display: block; margin-inline: auto; }
.fit { display: flex; gap: 8px; margin-top: 10px; justify-content: center; }
.fit button { padding: 4px 12px; border-radius: 7px; border: 1px solid #33333d; background: #1c1c26; color: #aaa; cursor: pointer; }
.fit button.on { border-color: #7aa2f7; color: #7aa2f7; }
.row { display: flex; gap: 10px; flex-wrap: wrap; }
.btn { padding: 8px 14px; border-radius: 9px; border: 1px solid #33333d; background: #1e1e29; color: #e8e8ec; cursor: pointer; font: inherit; }
.btn:hover { border-color: #555; }
.btn.primary { background: #2f4a7a; border-color: #3f62a8; font-weight: 600; }
.btn.primary:hover { background: #3a5a96; }
.btn.danger:hover { border-color: #a33; color: #faa; }
#preview { width: 100%; border-radius: 12px; border: 1px solid #232330; background: #000; min-height: 200px; object-fit: contain; }
.actions { display: flex; gap: 10px; margin-top: 12px; flex-wrap: wrap; }
#editor { width: 100%; height: calc(100vh - 220px); background: #12121a; color: #dcdce6; border: 1px solid #232330; border-radius: 12px; outline: none; resize: vertical; padding: 14px 16px; font: 13px/1.55 ui-monospace, 'JetBrains Mono', monospace; tab-size: 4; }
#codebar { display: flex; gap: 12px; align-items: center; margin-top: 10px; }
#status { font-size: 13px; color: #888; flex: 1; }
#status.ok { color: #7dd87d; } #status.err { color: #ff8a8a; }
#toast { position: fixed; left: 50%; bottom: 24px; transform: translateX(-50%); background: #23232d; border: 1px solid #33333d; padding: 10px 18px; border-radius: 10px; opacity: 0; transition: opacity .2s; pointer-events: none; max-width: 80vw; }
#toast.err { border-color: #a33; color: #faa; }
#toast.show { opacity: 1; }
#dlg { position: fixed; inset: 0; background: #0009; display: none; place-items: center; }
#dlg.show { display: grid; }
#dlg .box { background: #17171f; border: 1px solid #2a2a36; border-radius: 14px; padding: 24px; width: min(420px, 92vw); }
#dlg h3 { margin: 0 0 6px; }
#dlg p { color: #999; font-size: 13px; margin: 0 0 14px; }
#dlg label { display: block; font-size: 13px; color: #999; margin: 12px 0 4px; }
#dlg select, #dlg input[type=text] { width: 100%; }
.effects { display: grid; grid-template-columns: repeat(auto-fill, minmax(110px, 1fr)); gap: 8px; }
.eff { padding: 10px 6px; border-radius: 10px; border: 1px solid #2a2a36; background: #14141c; color: #c9c9d4; cursor: pointer; text-align: center; font-size: 13px; }
.eff small { display: block; color: #777; font-size: 11px; margin-top: 3px; }
.eff.on { border-color: #7aa2f7; color: #fff; background: #7aa2f718; }
.dropbig { border: 2px dashed #3a3a4a; border-radius: 12px; padding: 26px 10px; text-align: center; color: #999; cursor: pointer; }
.dropbig.over { border-color: #7aa2f7; color: #7aa2f7; background: #7aa2f711; }
#dlgerr { color: #ff8a8a; font-size: 13px; margin-top: 10px; min-height: 1em; }
</style>
</head>
<body>
<header>
  <h1>bruma studio</h1>
  <select id=pkg></select>
  <div id=tabs>
    <button class="tab on" data-p="design">Design</button>
    <button class="tab" data-p="code">Code</button>
  </div>
  <span id=hint>the running wallpaper is the live preview</span>
</header>
<main>
<section class="panel on" id=p-design>
  <div class=cols>
    <div>
      <div class=card>
        <h2>Layer stack</h2>
        <ul id=layerList></ul>
        <div class=lrow style="margin-top:8px">
          <select id=effPick style="flex:1"></select>
          <button class=btn id=addLayer>Add layer</button>
        </div>
      </div>
      <div class=card>
        <h2>Layer settings</h2>
        <div id=laySet><em>pick a layer above</em></div>
      </div>
      <div class=card>
        <h2>Wallpaper params</h2>
        <div id=params></div>
      </div>
      <div class=card>
        <h2>Photo layer</h2>
        <div id=texbox></div>
      </div>
    </div>
    <div>
      <img id=preview alt="preview">
      <div class=actions>
        <button class="btn primary" id=composeBtn>Compose effect over photo…</button>
        <button class=btn id=shotBtn>Upload preview</button>
        <button class=btn id=actBtn>Set as wallpaper</button>
        <button class="btn danger" id=delBtn>Uninstall</button>
      </div>
      <input type=file id=shotFile accept="image/*" hidden>
    </div>
  </div>
</section>
<section class="panel" id=p-code>
  <div class=card>
    <h2>Shader (WGSL) — optional, the Design tab is enough</h2>
    <textarea id=editor spellcheck=false></textarea>
    <div id=codebar>
      <span id=status></span>
      <button class="btn primary" id=save>Apply (Ctrl-S)</button>
    </div>
  </div>
</section>
</main>
<div id=toast></div>
<div id=dlg>
  <div class=box>
    <h3>Compose</h3>
    <p>One photo as the base layer, one effect over it. Everything stays editable afterwards (sliders, Code tab).</p>
    <label>1 · The photo (drag it here)</label>
    <div class=dropbig id=photoDrop>Drop an image or click to choose</div>
    <input type=file id=photoFile accept="image/*" hidden>
    <label>2 · The effect over it</label>
    <div class=effects id=effects></div>
    <label>3 · Photo fit</label>
    <div class=fit style="justify-content:flex-start">
      <button data-f="cover" class=on>cover (fill)</button>
      <button data-f="contain">contain (fit)</button>
    </div>
    <div class=row style="margin-top:18px">
      <button class="btn primary" id=goCompose style="flex:1">Compose now</button>
      <button class=btn id=cancelDlg>Cancel</button>
    </div>
    <div id=dlgerr></div>
  </div>
</div>
<script>
const $ = id => document.getElementById(id);
const pkgSel = $('pkg'), editor = $('editor'), status = $('status'), save = $('save'),
      toast = $('toast'), preview = $('preview');
let current = null, dirty = false, saveTimer = null, pickedEffect = 'water-cursor',
    pickedFit = 'cover', photoBytes = null, photoName = '';

function say(msg, cls) { status.textContent = msg; status.className = cls || ''; }
let toastTimer;
function pop(msg, err) {
  toast.textContent = msg; toast.className = 'show' + (err ? ' err' : '');
  clearTimeout(toastTimer); toastTimer = setTimeout(() => toast.className = '', 2800);
}

async function api(path, opts) {
  const r = await fetch(path, opts);
  const text = await r.text();
  let j = {}; try { j = JSON.parse(text); } catch { j = { error: text }; }
  if (!r.ok || j.error) throw new Error(j.error || r.statusText);
  return j;
}

/* ---- tabs ---- */
document.querySelectorAll('.tab').forEach(t => t.onclick = () => {
  document.querySelectorAll('.tab').forEach(x => x.classList.toggle('on', x === t));
  document.querySelectorAll('.panel').forEach(p => p.classList.toggle('on', p.id === 'p-' + t.dataset.p));
  if (t.dataset.p === 'design') loadAll();
});

/* ---- packages ---- */
async function loadPackages(keep) {
  const { packages } = await api('/api/packages');
  pkgSel.innerHTML = '';
  for (const p of packages) {
    const o = document.createElement('option');
    o.value = p.name; o.textContent = `${p.title} (${p.name})`;
    pkgSel.appendChild(o);
  }
  if (keep && packages.some(p => p.name === keep)) pkgSel.value = keep;
  current = pkgSel.value;
}

/* ---- sliders ---- */
async function loadParams() {
  const box = $('params'); box.innerHTML = '';
  const { params } = await api('/api/params?name=' + encodeURIComponent(current));
  if (!params.length) { box.innerHTML = '<i style="color:#666">this effect has no parameters</i>'; return; }
  for (const p of params) {
    const g = document.createElement('div'); g.className = 'pgroup';
    const lab = document.createElement('label');
    const name = document.createElement('span'); name.textContent = p.label || p.name;
    const val = document.createElement('span'); val.className = 'val'; val.textContent = p.value.toFixed(2);
    lab.append(name, val);
    const range = document.createElement('input');
    range.type = 'range'; range.min = 0; range.max = 1; range.step = 0.01; range.value = p.value;
    range.oninput = () => {
      val.textContent = (+range.value).toFixed(2);
      clearTimeout(saveTimer);
      saveTimer = setTimeout(() => setParam(p.name, +range.value), 160);
    };
    g.append(lab, range); box.appendChild(g);
  }
}

async function setParam(name, value) {
  try {
    await api('/api/params', { method: 'POST', headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ name: current, param: name, value }) });
    say(`${name} = ${value.toFixed(2)} → live`, 'ok');
  } catch (e) { say(e.message, 'err'); }
}

/* ---- texture layer ---- */
async function loadTexture() {
  const box = $('texbox'); box.innerHTML = '';
  const { textures } = await api('/api/textures?name=' + encodeURIComponent(current));
  const zone = document.createElement('div');
  zone.className = 'tzone';
  const render = (tex) => {
    zone.innerHTML = '';
    if (tex) {
      const img = document.createElement('img');
      img.src = tex.url + '&t=' + Date.now();
      zone.appendChild(img);
    }
    zone.appendChild(document.createTextNode(tex ? 'Drop to replace the photo' : 'Drop a photo to add the layer'));
    const fit = document.createElement('div'); fit.className = 'fit';
    for (const f of ['cover', 'contain']) {
      const b = document.createElement('button');
      b.textContent = f;
      b.classList.toggle('on', tex && tex.fit === f);
      b.onclick = () => { if (tex) setTexture(null, f); };
      fit.appendChild(b);
    }
    zone.appendChild(fit);
  };
  render(textures[0]);
  zone.onclick = () => pickImage(bytes => setTexture(bytes, null));
  zone.ondragover = e => { e.preventDefault(); zone.classList.add('over'); };
  zone.ondragleave = () => zone.classList.remove('over');
  zone.ondrop = e => {
    e.preventDefault(); zone.classList.remove('over');
    const f = [...e.dataTransfer.files].find(f => f.type.startsWith('image/'));
    if (f) pickImage(bytes => setTexture(bytes, null), f);
  };
  box.appendChild(zone);
}

function pickImage(cb, file) {
  const f = file || (() => { const i = document.createElement('input'); i.type = 'file'; i.accept = 'image/*';
    i.onchange = () => i.files[0] && readImg(i.files[0], cb); i.click(); return null; })();
  if (f) readImg(f, cb);
}
function readImg(file, cb) {
  const r = new FileReader();
  r.onload = () => cb(new Uint8Array(r.result));
  r.readAsArrayBuffer(file);
}

async function setTexture(bytes, fit) {
  try {
    const q = fit ? `&fit=${fit}` : '&fit=cover';
    await api('/api/textures?name=' + encodeURIComponent(current) + q,
      { method: 'PUT', body: bytes });
    pop('photo layer updated — the daemon re-arms on the next SIGHUP or restart');
    loadTexture();
  } catch (e) { pop(e.message, true); }
}

/* ---- preview + actions ---- */
async function loadPreview() {
  preview.src = '/api/preview?name=' + encodeURIComponent(current) + '&t=' + Date.now();
}
$('shotBtn').onclick = () => $('shotFile').click();
$('shotFile').onchange = async () => {
  const f = $('shotFile').files[0]; if (!f) return;
  const buf = new Uint8Array(await f.arrayBuffer());
  try {
    await api('/api/preview?name=' + encodeURIComponent(current), { method: 'PUT', body: buf });
    pop('preview updated'); loadPreview();
  } catch (e) { pop(e.message, true); }
};
$('actBtn').onclick = async () => {
  try { await api('/api/activate', { method: 'POST', headers: {'content-type':'application/json'}, body: JSON.stringify({name: current}) }); pop(`${current} is now your wallpaper`); }
  catch (e) { pop(e.message, true); }
};
$('delBtn').onclick = async () => {
  if (!confirm(`Uninstall ${current}?`)) return;
  try {
    const { version } = await (await fetch('/api/packages')).json()
      .then(d => d.packages.find(p => p.name === current));
    await api('/api/uninstall', { method: 'POST', headers: {'content-type':'application/json'}, body: JSON.stringify({name: current, version}) });
    pop('uninstalled'); await loadPackages(); loadAll();
  } catch (e) { pop(e.message, true); }
};

/* ---- compose dialog ---- */
const EFFECTS = [
  // Compose = your photo as the base layer + a transparent effect pass
  // on top. Only the photo-overlay family is offered (water-cursor and
  // water-photo reshape the photo itself; the rest composite over it).
  ['water-cursor', 'Calm water', 'wake + rings under the cursor'],
  ['water-photo', 'Still water', 'gentle ripples over the photo'],
  ['parallax-photo', 'Parallax', 'aurora depth layers, mouse-reactive'],
  ['fog-photo', 'Fog', 'drifting mist over the photo'],
  ['waves-photo', 'Waves', 'soft concentric ripples'],
  ['trail-photo', 'Light trail', 'glowing cursor path'],
];
(function fillEffects() {
  const box = $('effects');
  for (const [id, label, desc] of EFFECTS) {
    const b = document.createElement('button');
    b.className = 'eff' + (id === pickedEffect ? ' on' : '');
    b.innerHTML = `${label}<small>${desc}</small>`;
    b.onclick = () => { pickedEffect = id; box.querySelectorAll('.eff').forEach(x => x.classList.toggle('on', x === b)); };
    box.appendChild(b);
  }
})();
document.querySelectorAll('#dlg .fit button').forEach(b => b.onclick = () => {
  pickedFit = b.dataset.f;
  document.querySelectorAll('#dlg .fit button').forEach(x => x.classList.toggle('on', x === b));
});
$('composeBtn').onclick = () => { $('dlg').classList.add('show'); $('dlgerr').textContent = ''; };
$('cancelDlg').onclick = () => $('dlg').classList.remove('show');
$('photoDrop').onclick = () => $('photoFile').click();
$('photoDrop').ondragover = e => { e.preventDefault(); $('photoDrop').classList.add('over'); };
$('photoDrop').ondragleave = () => $('photoDrop').classList.remove('over');
$('photoDrop').ondrop = e => {
  e.preventDefault(); $('photoDrop').classList.remove('over');
  const f = [...e.dataTransfer.files].find(f => f.type.startsWith('image/'));
  if (f) { pickComposePhoto(f); }
};
$('photoFile').onchange = () => $('photoFile').files[0] && pickComposePhoto($('photoFile').files[0]);
function pickComposePhoto(f) {
  photoName = f.name; readImg(f, b => {
    photoBytes = b;
    $('photoDrop').textContent = `✓ ${f.name} (${(b.length/1024).toFixed(0)} KB) — drop to replace`;
  });
}
$('goCompose').onclick = async () => {
  if (!photoBytes) { $('dlgerr').textContent = 'choose a photo first'; return; }
  $('dlgerr').textContent = 'composing…';
  try {
    // Chunked conversion: spreading N bytes as function arguments
    // throws RangeError "too many function arguments" on real photos.
    // The chunk must be a multiple of 3 or each btoa() call pads with
    // '=' mid-stream and the server-side decoder rejects the result.
    let b64 = '';
    const CH = 32766;
    for (let i = 0; i < photoBytes.length; i += CH) {
      b64 += btoa(String.fromCharCode(...photoBytes.subarray(i, i + CH)));
    }
    const j = await api('/api/compose', { method: 'POST', headers: {'content-type':'application/json'},
      body: JSON.stringify({ name: current, effect: pickedEffect, fit: pickedFit, photo_b64: b64 }) });
    $('dlg').classList.remove('show');
    pop(`composed: ${j.effect} over your photo — activate it to see it live`);
    loadAll();
  } catch (e) { $('dlgerr').textContent = e.message; }
};

/* ---- layer stack (L1) ---- */
let CATALOG = [];
let layersDoc = { base: null, layers: [] };
let pickedLayer = -1;
let layersTimer = null;

async function loadCatalog() {
  if (CATALOG.length) return;
  try {
    const j = await api('/api/catalog');
    CATALOG = j.effects;
    $('effPick').innerHTML = CATALOG.map(e => `<option value="${e.id}">${e.label}</option>`).join('');
  } catch {}
}

async function loadLayers() {
  if (!current) return;
  try {
    layersDoc = await api('/api/layers?name=' + encodeURIComponent(current));
  } catch { layersDoc = { base: null, layers: [] }; }
  pickedLayer = -1;
  renderLayers();
}

function renderLayers() {
  const ul = $('layerList');
  ul.innerHTML = '';
  const mk = (label, meta, cls, btns, onpick) => {
    const li = document.createElement('li'); li.className = 'llayer ' + cls;
    const nm = document.createElement('span'); nm.className = 'lname'; nm.textContent = label;
    const mt = document.createElement('span'); mt.className = 'lmeta'; mt.textContent = meta;
    li.append(nm, mt);
    for (const [txt, fn, title] of btns) {
      const b = document.createElement('button'); b.textContent = txt; if (title) b.title = title;
      b.onclick = ev => { ev.stopPropagation(); fn(); };
      li.appendChild(b);
    }
    li.onclick = onpick;
    ul.appendChild(li);
    return li;
  };
  mk('Photo (base)', layersDoc.base ? layersDoc.base.fit : 'none',
     'base' + (pickedLayer === -2 ? ' on' : ''),
     [], () => { pickedLayer = -2; renderLayers(); });
  layersDoc.layers.forEach((l, i) => {
    mk(l.effect, `op ${l.opacity.toFixed(2)} · d ${l.depth.toFixed(2)}`,
       pickedLayer === i ? 'on' : '',
       [
         ['\u2191', () => moveLayer(i, -1), 'up'],
         ['\u2193', () => moveLayer(i, +1), 'down'],
         ['\u00d7', () => removeLayer(i), 'remove'],
       ],
       () => { pickedLayer = i; renderLayers(); });
  });
  renderLayerSettings();
}

function renderLayerSettings() {
  const box = $('laySet');
  box.innerHTML = '';
  const slider = (label, value, oninput) => {
    const wrap = document.createElement('div'); wrap.className = 'pgroup';
    const lab = document.createElement('label');
    const nm = document.createElement('span'); nm.textContent = label;
    const val = document.createElement('span'); val.className = 'val'; val.textContent = value.toFixed(2);
    lab.append(nm, val);
    const range = document.createElement('input'); range.type = 'range'; range.min = 0; range.max = 1; range.step = 0.01; range.value = value;
    range.oninput = () => { val.textContent = (+range.value).toFixed(2); oninput(+range.value); };
    wrap.append(lab, range);
    box.appendChild(wrap);
  };
  if (pickedLayer === -2) {
    if (layersDoc.base) {
      slider('Parallax depth', layersDoc.base.depth ?? 0, v => { layersDoc.base.depth = v; scheduleLayersSave(); });
      const fits = ['cover', 'contain'];
      const sel = document.createElement('select');
      sel.innerHTML = fits.map(f => `<option ${layersDoc.base.fit === f ? 'selected' : ''}>${f}</option>`).join('');
      sel.onchange = () => { layersDoc.base.fit = sel.value; scheduleLayersSave(); };
      box.appendChild(sel);
    } else { box.innerHTML = '<em>no photo — compose one (button on the right)</em>'; }
    return;
  }
  if (pickedLayer < 0) { box.innerHTML = '<em>pick a layer above</em>'; return; }
  const l = layersDoc.layers[pickedLayer];
  const eff = CATALOG.find(e => e.id === l.effect);
  slider('Opacity', l.opacity ?? 1, v => { l.opacity = v; scheduleLayersSave(); });
  slider('Parallax depth', l.depth ?? 0, v => { l.depth = v; scheduleLayersSave(); });
  if (eff) for (const p of eff.params) {
    slider(p.label, (l.params && l.params[p.name]) ?? p.default, v => {
      l.params = l.params || {}; l.params[p.name] = v; scheduleLayersSave();
    });
  }
}

function scheduleLayersSave() {
  clearTimeout(layersTimer);
  layersTimer = setTimeout(saveLayers, 350);
}
async function saveLayers() {
  if (!current) return;
  try {
    await api('/api/layers?name=' + encodeURIComponent(current), {
      method: 'PUT', headers: { 'content-type': 'application/json' },
      body: JSON.stringify(layersDoc),
    });
    pop('layers applied — live');
    loadParams();
  } catch (e) { pop(e.message); }
}

$('addLayer').onclick = () => {
  const id = $('effPick').value;
  if (!id) return;
  layersDoc.layers.push({ effect: id, opacity: 1, depth: 0, params: {} });
  pickedLayer = layersDoc.layers.length - 1;
  renderLayers(); scheduleLayersSave();
};
function removeLayer(i) {
  layersDoc.layers.splice(i, 1);
  pickedLayer = -1;
  renderLayers(); scheduleLayersSave();
}
function moveLayer(i, dir) {
  const j = i + dir;
  if (j < 0 || j >= layersDoc.layers.length) return;
  [layersDoc.layers[i], layersDoc.layers[j]] = [layersDoc.layers[j], layersDoc.layers[i]];
  pickedLayer = j;
  renderLayers(); scheduleLayersSave();
}

/* ---- code tab (optional) ---- */
async function loadShader() {
  const r = await fetch('/api/shader?name=' + encodeURIComponent(current));
  editor.value = await r.text();
  dirty = false; say('loaded ' + current, 'ok');
}
async function applyShader() {
  save.disabled = true;
  try {
    await api('/api/shader?name=' + encodeURIComponent(current), { method: 'PUT', body: editor.value });
    dirty = false; say('applied — daemon hot-reloads in a moment', 'ok');
  } catch (e) { say(e.message.split('\n').slice(0, 3).join(' | '), 'err'); }
  save.disabled = false;
}
editor.addEventListener('input', () => { if (!dirty) { dirty = true; say('edited — Ctrl-S to apply'); } });
editor.addEventListener('keydown', e => {
  if ((e.ctrlKey || e.metaKey) && e.key === 's') { e.preventDefault(); if (dirty) applyShader(); }
  else if (e.key === 'Tab') {
    e.preventDefault();
    const s = editor.selectionStart;
    editor.setRangeText('    ', s, editor.selectionEnd, 'end');
  }
});
save.onclick = applyShader;

/* ---- boot ---- */
function loadAll() { if (!current) return; loadParams(); loadTexture(); loadPreview(); loadShader(); loadLayers(); loadCatalog(); }
pkgSel.onchange = () => { current = pkgSel.value; loadAll(); };
(async () => {
  await loadPackages();
  if (current) loadAll();
})();
</script>
</body>
</html>
"#;
