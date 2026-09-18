// The embedded studio UI (kept in a separate file only to keep the
// server logic readable; included verbatim into studio.rs).

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
header { display: flex; align-items: center; gap: 14px; padding: 12px 20px; border-bottom: 1px solid #22222c; }
h1 { font-size: 16px; font-weight: 600; margin: 0; }
select, input { background: #1c1c26; color: #e8e8ec; border: 1px solid #33333d; border-radius: 8px; padding: 6px 10px; font: inherit; }
#hint { color: #888; font-size: 13px; }
main { display: grid; grid-template-columns: 260px 1fr; min-height: calc(100vh - 57px); }
aside { border-right: 1px solid #22222c; padding: 16px; overflow-y: auto; }
aside h2 { font-size: 12px; text-transform: uppercase; letter-spacing: .08em; color: #777; margin: 0 0 10px; }
.tgroup { display: flex; flex-direction: column; gap: 6px; margin-bottom: 24px; }
.tbtn { text-align: left; padding: 8px 12px; border-radius: 8px; border: 1px solid #2a2a36; background: #17171f; color: #c9c9d4; cursor: pointer; }
.tbtn:hover { border-color: #7aa2f7; color: #fff; }
.pgroup { margin-bottom: 14px; }
.pgroup label { display: flex; justify-content: space-between; font-size: 13px; margin-bottom: 4px; }
.pgroup .val { color: #7aa2f7; font-variant-numeric: tabular-nums; }
input[type=range] { width: 100%; accent-color: #7aa2f7; }
#editor { width: 100%; height: calc(100vh - 57px); background: #12121a; color: #dcdce6; border: 0; outline: none; resize: none; padding: 16px 20px; font: 13px/1.55 ui-monospace, 'JetBrains Mono', monospace; tab-size: 4; }
#savebar { position: fixed; right: 24px; bottom: 24px; display: flex; gap: 10px; align-items: center; }
#status { font-size: 13px; color: #888; }
#status.ok { color: #7dd87d; } #status.err { color: #ff8a8a; }
button.primary { padding: 9px 18px; border-radius: 10px; border: 1px solid #3f62a8; background: #2f4a7a; color: #fff; cursor: pointer; font-weight: 600; }
button.primary:hover { background: #3a5a96; }
button.primary:disabled { opacity: .5; cursor: default; }
#newdlg { position: fixed; inset: 0; background: #0009; display: none; place-items: center; }
#newdlg.show { display: grid; }
#newdlg .box { background: #17171f; border: 1px solid #2a2a36; border-radius: 14px; padding: 24px; width: 340px; }
#newdlg h3 { margin: 0 0 14px; }
#newdlg label { display: block; font-size: 13px; color: #999; margin: 10px 0 4px; }
#newdlg input, #newdlg select { width: 100%; }
#newdlg .row { display: flex; gap: 10px; margin-top: 18px; }
#newerr { color: #ff8a8a; font-size: 13px; margin-top: 8px; min-height: 1em; }
</style>
</head>
<body>
<header>
  <h1>bruma studio</h1>
  <select id=pkg></select>
  <span id=hint>the running wallpaper is the live preview</span>
</header>
<main>
  <aside>
    <h2>New from template</h2>
    <div class=tgroup id=templates></div>
    <h2>Parameters (live)</h2>
    <div id=params></div>
  </aside>
  <div style="position:relative">
    <textarea id=editor spellcheck=false></textarea>
    <div id=savebar>
      <span id=status></span>
      <button class=primary id=save>Apply (Ctrl-S)</button>
    </div>
  </div>
</main>
<div id=newdlg>
  <div class=box>
    <h3>New wallpaper</h3>
    <label>Name (package identity)</label>
    <input id=newname placeholder="my-aurora">
    <label>Template</label>
    <select id=newtpl></select>
    <div class=row>
      <button class=primary id=newgo style="flex:1">Create</button>
      <button class=tbtn id=newcancel style="flex:0">Cancel</button>
    </div>
    <div id=newerr></div>
  </div>
</div>
<script>
const $ = id => document.getElementById(id);
const pkgSel = $('pkg'), editor = $('editor'), status = $('status'), save = $('save');
let current = null, dirty = false, saveTimer = null;

function say(msg, cls) { status.textContent = msg; status.className = cls || ''; }

async function api(path, opts) {
  const r = await fetch(path, opts);
  const text = await r.text();
  let j = {}; try { j = JSON.parse(text); } catch { j = { error: text }; }
  if (!r.ok || j.error) throw new Error(j.error || r.statusText);
  return j;
}

async function loadPackages() {
  const { packages } = await api('/api/packages');
  pkgSel.innerHTML = '';
  for (const p of packages) {
    const o = document.createElement('option');
    o.value = p.name; o.textContent = `${p.title} (${p.name})`;
    pkgSel.appendChild(o);
  }
}

async function loadShader() {
  current = pkgSel.value;
  if (!current) return;
  const r = await fetch('/api/shader?name=' + encodeURIComponent(current));
  editor.value = await r.text();
  dirty = false; say('loaded ' + current, 'ok');
  loadParams(); loadManifest();
}

async function loadParams() {
  const { params } = await api('/api/params?name=' + encodeURIComponent(current));
  const box = $('params'); box.innerHTML = '';
  for (const p of params) {
    const g = document.createElement('div'); g.className = 'pgroup';
    const lab = document.createElement('label');
    lab.innerHTML = `<span></span><span class='val'></span>`;
    lab.firstChild.textContent = p.label || p.name;
    const val = lab.lastChild; val.textContent = p.value.toFixed(2);
    const range = document.createElement('input');
    range.type = 'range'; range.min = 0; range.max = 1; range.step = 0.01; range.value = p.value;
    range.oninput = () => {
      val.textContent = (+range.value).toFixed(2);
      clearTimeout(saveTimer);
      saveTimer = setTimeout(() => setParam(p.name, +range.value), 180);
    };
    g.appendChild(lab); g.appendChild(range); box.appendChild(g);
  }
}

async function setParam(name, value) {
  try {
    await api('/api/params', { method: 'POST', headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ name: current, param: name, value }) });
    say(`${name} = ${value.toFixed(2)} → live`, 'ok');
  } catch (e) { say(e.message, 'err'); }
}

async function loadManifest() {
  try {
    const m = await api('/api/manifest?name=' + encodeURIComponent(current));
    document.title = `bruma studio — ${m.title}`;
  } catch {}
}

async function apply() {
  save.disabled = true;
  try {
    await api('/api/shader?name=' + encodeURIComponent(current), { method: 'PUT', body: editor.value });
    dirty = false; say('applied — daemon hot-reloads in a moment', 'ok');
  } catch (e) { say(e.message.split('\n').slice(0, 3).join(' | '), 'err'); }
  save.disabled = false;
}

editor.addEventListener('input', () => {
  if (!dirty) { dirty = true; say('edited — Ctrl-S to apply'); }
});
editor.addEventListener('keydown', e => {
  if ((e.ctrlKey || e.metaKey) && e.key === 's') { e.preventDefault(); if (dirty) apply(); }
  else if (e.key === 'Tab') {
    e.preventDefault();
    const s = editor.selectionStart;
    editor.setRangeText('    ', s, editor.selectionEnd, 'end');
  }
});
save.onclick = apply;

async function fillTemplates() {
  const templates = ['waves', 'fog', 'water', 'water-photo', 'trail', 'parallax', 'water-cursor'];
  const box = $('templates');
  for (const t of templates) {
    const b = document.createElement('button');
    b.className = 'tbtn'; b.textContent = t;
    b.onclick = () => { $('newtpl').value = t; $('newdlg').classList.add('show'); $('newname').focus(); };
    box.appendChild(b);
  }
  const sel = $('newtpl');
  sel.innerHTML = '';
  for (const t of templates) { const o = document.createElement('option'); o.value = t; o.textContent = t; sel.appendChild(o); }
}

$('newgo').onclick = async () => {
  $('newerr').textContent = '';
  try {
    const j = await api('/api/create', { method: 'POST', headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ name: $('newname').value.trim(), template: $('newtpl').value }) });
    $('newdlg').classList.remove('show');
    await loadPackages();
    pkgSel.value = j.name;
    loadShader();
  } catch (e) { $('newerr').textContent = e.message; }
};
$('newcancel').onclick = () => $('newdlg').classList.remove('show');

pkgSel.onchange = () => { if (dirty && !confirm('Discard unsaved shader edits?')) { pkgSel.value = current; return; } loadShader(); };

(async () => {
  await loadPackages(); await fillTemplates();
  if (pkgSel.options.length) { loadShader(); } else { say('create a wallpaper from a template', ''); }
})();
</script>
</body>
</html>
"#;
