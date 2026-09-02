/*
 * One violation of every rule in javascript-airbnb.toml, the compliant form
 * beside it where that's cheap. Check with:
 *   python3 scripts/ruleset-check.py rulesets/javascript-airbnb.toml rulesets/samples/javascript-airbnb.js
 */

// [no-var]
var count = 0;

// compliant
const total = 0;

// [no-arguments-object]
function sum() {
  let out = 0;
  for (let i = 0; i < arguments.length; i++) {
    out += arguments[i];
  }
  return out;
}

// compliant: rest syntax instead
function sumOk(...values) {
  return values.reduce((a, b) => a + b, 0);
}

// [no-new-function]
const add = new Function("a", "b", "return a + b");

// compliant
const addOk = (a, b) => a + b;

// [no-namespace-import]
import * as helpers from "./helpers.js";

// compliant
import { formatDate } from "./helpers.js";

// [no-prototype-mutation]
function Animal(name) {
  this.name = name;
}
Animal.prototype.speak = function speak() {
  return `${this.name} makes a noise.`;
};

// compliant
class AnimalOk {
  constructor(name) {
    this.name = name;
  }

  speak() {
    return `${this.name} makes a noise.`;
  }
}

// [default-params-last]
function greet(greeting = "hello", name) {
  return `${greeting}, ${name}`;
}

// compliant
function greetOk(name, greeting = "hello") {
  return `${greeting}, ${name}`;
}

// [no-nested-ternary]
function sizeLabel(n) {
  return n > 100 ? "large" : n > 10 ? "medium" : "small";
}

// compliant
function sizeLabelOk(n) {
  if (n > 100) return "large";
  if (n > 10) return "medium";
  return "small";
}

// [no-function-in-block]
function build(flag) {
  if (flag) {
    function helper() {
      return "built";
    }
    return helper();
  }
  return null;
}

// compliant: assign a function expression instead
function buildOk(flag) {
  let helper;
  if (flag) {
    helper = function helper() {
      return "built";
    };
    return helper();
  }
  return null;
}
