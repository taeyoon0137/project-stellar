export const contexts = Object.freeze({
  keys: Object.freeze({
    "00000000000000000000000000000066": Object.freeze({ name: "label", defaultValue: (read) => "기본" }),
  }),
  provisions: Object.freeze([
    Object.freeze({ element: "s-00000000000000000000000000000009", key: "00000000000000000000000000000066", depths: [1, 2], value: (read, outer) => "메모" }),
  ]),
  consumers: Object.freeze([
    Object.freeze({ element: "s-0000000000000000000000000000000a", property: "00000000-0000-0000-0000-00000000000a.4", key: "00000000000000000000000000000066", from: "s-00000000000000000000000000000009" }),
  ]),
});
