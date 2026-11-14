// 테스트에서만 주입합니다. 제품 HTML에는 script가 없습니다.
const readings = {};
const failures = [];
const id = n => 's-' + n.toString(16).padStart(12, '0') + '70008000' + n.toString(16).padStart(12, '0');
const element = n => document.getElementById(id(n));
const equal = (actual, expected, label) => {
  if (actual !== expected) failures.push(`${label}: ${actual} != ${expected}`);
};
const near = (actual, expected, label) => {
  if (Math.abs(actual - expected) > 1 / 64) failures.push(`${label}: ${actual} != ${expected}`);
};
const inspect = (n, rect) => {
  const el = element(n);
  if (!el) throw new Error(`missing node ${n}`);
  const style = getComputedStyle(el);
  const bounds = el.getBoundingClientRect();
  const actual = [bounds.x, bounds.y, bounds.width, bounds.height];
  actual.forEach((v, i) => near(v, rect[i], `node ${n} rect[${i}]`));
  readings[n] = {
    rect: actual, width: style.width, height: style.height, boxSizing: style.boxSizing,
    transform: style.transform, transformOrigin: style.transformOrigin,
    borderWidth: style.borderTopWidth, borderStyle: style.borderTopStyle,
    borderColor: style.borderTopColor, background: style.backgroundColor,
    radii: [style.borderTopLeftRadius, style.borderTopRightRadius, style.borderBottomRightRadius, style.borderBottomLeftRadius],
    overflow: style.overflow, overflowClipMargin: style.overflowClipMargin,
    position: style.position, zIndex: style.zIndex, display: style.display
  };
  equal(style.boxSizing, 'border-box', `node ${n} boxSizing`);
  equal(style.transformOrigin, '0px 0px', `node ${n} origin`);
  equal(style.zIndex, '0', `node ${n} stacking context`);
  return readings[n];
};
try {
  equal(devicePixelRatio, 1, 'DPR');
  equal(CSS.supports('overflow-clip-margin', 'border-box'), true, 'border-box clip capability');
  equal(CSS.supports('color', 'color(display-p3 0.1 0.2 0.3 / 0.8)'), true, 'P3 capability');
  if (selectedView === 3) {
    const root = inspect(10, [0, 0, 640, 400]);
    equal(root.width, '640px', 'root width');
    equal(root.height, '400px', 'root height');
    equal(root.borderWidth, '4px', 'root border');
    equal(root.borderStyle, 'solid', 'root border style');
    for (const side of ['Top', 'Right', 'Bottom', 'Left']) {
      equal(getComputedStyle(element(10))[`border${side}Width`], '4px', `root ${side} border width`);
      equal(getComputedStyle(element(10))[`border${side}Color`], 'color(srgb 0.1 0.2 0.3)', `root ${side} border color`);
    }
    equal(root.borderColor, 'color(srgb 0.1 0.2 0.3)', 'root border color');
    equal(root.background, 'color(srgb 0.95 0.96 0.98)', 'root background');
    equal(root.radii.join(','), '8px,16px,24px,32px', 'root radius');
    equal(root.transform, 'matrix(1, 0, 0, 1, 0, 0)', 'root canvas offset removed');
    equal(root.overflow, 'clip', 'root clip');
    equal(root.overflowClipMargin, 'border-box', 'outer clip edge');
    const card = inspect(12, [32, 48, 240, 160]);
    equal(card.borderWidth, '3px', 'card border');
    equal(card.background, 'color(display-p3 0.1 0.2 0.3 / 0.8)', 'card P3/RGBA');
    equal(card.borderColor, 'color(display-p3 0.8 0.2 0.1 / 0.5)', 'border P3/RGBA');
    equal(card.radii.join(','), '12px,20px,28px,36px', 'individual radii');
    const bar = inspect(15, [42, 60, 80, 24]);
    equal(bar.borderWidth, '1px', 'nested border');
    equal(bar.background, 'rgb(255, 0, 0)', 'hex color');
    equal(bar.overflow, 'visible', 'disabled clip');
    inspect(13, [63, 72, 100, 40]);
    equal(document.elementFromPoint(80, 80).id, id(13), 'OrderKey stacking after Fragment flatten');
    equal(inspect(17, [0, 0, 0, 0]).display, 'none', 'hidden Fragment descendant');
    inspect(18, [0, 100, 8, 8]);
    equal(document.elementFromPoint(1, 103).id, id(18), 'clip includes parent border area');
    inspect(19, [630, 300, 30, 40]);
    equal(document.elementFromPoint(635, 310).id, id(19), 'inside root clip');
    equal(document.elementFromPoint(645, 310).id === id(19), false, 'outside root clip');
    for (const n of [11, 14, 16, 20]) equal(element(n), null, `node ${n} absent`);
  } else {
    const root = inspect(20, [0, 0, 300, 240]);
    equal(root.width, '240px', 'root untransformed width');
    equal(root.height, '160px', 'root untransformed height');
    equal(root.transform, 'matrix(1.25, 0, 0, 1.5, 0, 0)', 'root linear transform retained');
    equal(inspect(21, [100, 37.5, 25, 60]).transform, 'matrix(0, 1, -1, 0, 0, 0)', 'rotation retained');
    equal(inspect(22, [187.5, 150, 75, 45]).transform, 'matrix(-1, 0, 0, 1, 0, 0)', 'mirror retained');
    const shear = inspect(23, [25, 150, 65, 51]);
    equal(shear.transform, 'matrix(1, 0.25, 0.5, 1, 0, 0)', 'shear retained');
    equal(shear.background, 'color(display-p3 1.2 -0.1 0.3 / 0.25)', 'extended RGB channels and alpha retained');
    equal(element(10), null, 'View A excluded');
  }
} catch (error) {
  failures.push(String(error));
}
const report = document.createElement('pre');
report.id = 'stellar-web-probe';
report.textContent = JSON.stringify({ok: failures.length === 0, failures, readings});
document.body.appendChild(report);
