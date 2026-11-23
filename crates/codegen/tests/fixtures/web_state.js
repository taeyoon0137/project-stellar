const root = document.getElementById("s-00000000000000000000000000000001");
const state = {
  "00000000000000000000000000000033": "",
  "00000000000000000000000000000034": false,
};
const read = {
  state: (id) => state[id],
  equals: (a, b) => JSON.stringify(a) === JSON.stringify(b),
  clamp: (v, lo, hi) => Math.min(Math.max(v, lo), hi),
  concat: (...parts) => parts.join(""),
};
function text(value) {
  if (value !== null && typeof value === "object" && value.type === "enum") {
    return value.enumType + ":" + value.variant;
  }
  return String(value);
}
function commit(changes) {
  for (const [id, value] of changes) {
    state[id] = value;
    root.setAttribute("data-s-" + id, text(value));
  }
}
for (const id of Object.keys(state)) {
  root.setAttribute("data-s-" + id, text(state[id]));
}
document.getElementById("s-0000000000000000000000000000000c").addEventListener("click", () => {
  if ((!read.state("00000000000000000000000000000034"))) {
    commit([["00000000000000000000000000000034", true]]);
    root.dispatchEvent(new CustomEvent("stellar:000000000000000000000000000000c9", { bubbles: true }));
    return;
  }
});
document.getElementById("s-0000000000000000000000000000000b").addEventListener("input", () => {
  if (true) {
    commit([["00000000000000000000000000000033", "입력"]]);
    return;
  }
});
document.getElementById("s-0000000000000000000000000000000f").addEventListener("click", () => {
  if (true) {
    commit([["00000000000000000000000000000034", false]]);
    return;
  }
});
