// A tiny stand-in for the design canvas's runtime, so the mockups in this
// folder open as plain pages: it runs each page's Component.renderVals(),
// then expands {{holes}}, <sc-for> and <sc-if> in the <x-dc> template.
class DCLogic {
  constructor(props) { this.props = props; this.state = {}; }
  setState(s) { Object.assign(this.state, s); }
}

function lookup(scope, path) {
  path = path.trim();
  if (path === 'true') return true;
  if (path === 'false') return false;
  return path.split('.').reduce((v, k) => (v == null ? undefined : v[k]), scope);
}

function fill(text, scope) {
  return text.replace(/\{\{([^}]+)\}\}/g, (_, p) => {
    const v = lookup(scope, p);
    return v == null ? '' : String(v);
  });
}

function whole(text, scope) {
  const m = /^\s*\{\{([^}]+)\}\}\s*$/.exec(text || '');
  return m ? lookup(scope, m[1]) : text;
}

function expand(node, scope) {
  if (node.nodeType === Node.TEXT_NODE) {
    return [document.createTextNode(fill(node.textContent, scope))];
  }
  if (node.nodeType !== Node.ELEMENT_NODE) return [];
  const tag = node.tagName.toLowerCase();
  const kids = (s) => [...node.childNodes].flatMap((c) => expand(c, s));
  if (tag === 'sc-for') {
    const list = whole(node.getAttribute('list'), scope) || [];
    const as = node.getAttribute('as') || 'item';
    return list.flatMap((item, i) => kids({ ...scope, [as]: item, $index: i }));
  }
  if (tag === 'sc-if') {
    return whole(node.getAttribute('value'), scope) ? kids(scope) : [];
  }
  const el = document.createElementNS(node.namespaceURI, node.tagName);
  for (const a of node.attributes) el.setAttribute(a.name, fill(a.value, scope));
  kids(scope).forEach((c) => el.appendChild(c));
  return [el];
}

document.addEventListener('DOMContentLoaded', () => {
  const script = document.querySelector('script[data-dc-script]');
  const declared = JSON.parse(script.getAttribute('data-props') || '{}');
  const props = {};
  for (const [k, v] of Object.entries(declared)) if (v && 'default' in v) props[k] = v.default;
  const Component = new Function('DCLogic', script.textContent + '\nreturn Component;')(DCLogic);
  const vals = new Component(props).renderVals();
  const root = document.querySelector('x-dc');
  const helmet = root.querySelector('helmet');
  if (helmet) {
    [...helmet.children].forEach((c) => document.head.appendChild(c));
    helmet.remove();
  }
  const out = [...root.childNodes].flatMap((c) => expand(c, vals));
  root.replaceWith(...out);
});
