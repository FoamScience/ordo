/*
 * One violation of every rule in typescript-clean-code.toml, the compliant
 * form beside it where that's cheap. Check with:
 *   python3 scripts/ruleset-check.py rulesets/typescript-clean-code.toml rulesets/samples/typescript-clean-code.ts
 */

// [few-function-arguments] more than 2 arguments
function createUser(name: string, email: string, age: number): void {
  console.log(name, email, age);
}

// compliant: an options object instead
function createUserOk(options: { name: string; email: string; age: number }): void {
  console.log(options);
}

// [no-boolean-flag-param] a bool parameter hides a second responsibility
function setUserActive(userId: string, isAdmin: boolean): void {
  console.log(userId, isAdmin);
}

// compliant: the intent has a name
type Role = "admin" | "member";
function setUserRole(userId: string, role: Role): void {
  console.log(userId, role);
}

// [throw-only-error] loses the stack trace
function parseAge(raw: string): number {
  const n = Number(raw);
  if (Number.isNaN(n)) {
    throw "not a number";
  }
  return n;
}

// compliant
function parseAgeOk(raw: string): number {
  const n = Number(raw);
  if (Number.isNaN(n)) {
    throw new Error("not a number");
  }
  return n;
}

// [no-typeof-check] / [no-instanceof-check] avoid type checking
class Money {
  amount: number;
  constructor(amount: number) {
    this.amount = amount;
  }
}

function describe(value: unknown): string {
  if (typeof value === "string") {
    return `string: ${value}`;
  }
  if (value instanceof Money) {
    return `money: ${value.amount}`;
  }
  return "unknown";
}

// [private-members] no accessor named — implicitly public
class Account {
  balance: number;
  private ledgerId: string;

  constructor(balance: number, ledgerId: string) {
    this.balance = balance;
    this.ledgerId = ledgerId;
  }
}

// [flat-control-flow] more than 2 nested conditionals
function classify(a: boolean, b: boolean, c: boolean): string {
  if (a) {
    if (b) {
      if (c) {
        return "abc";
      }
    }
  }
  return "other";
}

// compliant: return early instead of nesting
function classifyOk(a: boolean, b: boolean, c: boolean): string {
  if (!a || !b || !c) {
    return "other";
  }
  return "abc";
}

// [small-classes] over 50 lines, one responsibility too many
class OrderProcessor {
  id: string;
  items: string[] = [];
  taxRate: number = 0.2;
  shippingCost: number = 0;
  discount: number = 0;
  status: string = "pending";

  constructor(id: string) {
    this.id = id;
  }

  addItem(item: string): void {
    this.items.push(item);
  }

  removeItem(item: string): void {
    this.items = this.items.filter((i) => i !== item);
  }

  itemCount(): number {
    return this.items.length;
  }

  subtotal(): number {
    return this.items.length * 10;
  }

  tax(): number {
    return this.subtotal() * this.taxRate;
  }

  total(): number {
    return this.subtotal() + this.tax() + this.shippingCost - this.discount;
  }

  applyDiscount(amount: number): void {
    this.discount = amount;
  }

  setShipping(cost: number): void {
    this.shippingCost = cost;
  }

  markPaid(): void {
    this.status = "paid";
  }

  markShipped(): void {
    this.status = "shipped";
  }

  markDelivered(): void {
    this.status = "delivered";
  }

  cancel(): void {
    this.status = "cancelled";
  }

  isCancelled(): boolean {
    return this.status === "cancelled";
  }

  summary(): string {
    return `${this.id}: ${this.items.length} items, total ${this.total()}`;
  }

  auditLog(): string {
    return `order ${this.id} status=${this.status}`;
  }
}
