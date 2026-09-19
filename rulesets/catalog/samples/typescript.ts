// Exercises every rule in rulesets/catalog/javascript.toml.
export function loose(input: any): number {   // any
  let out = 0;
  try {
    out = eval(input);                        // eval
  } catch (e) {
  }                                           // empty-catch
  return out;
}

export function scoped(obj: object) {
  with (obj) {                                // with
    return 1;
  }
}
