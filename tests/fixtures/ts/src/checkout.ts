// Interface calls, callbacks, and functions passed as values.

import { double, pad } from "./format";
import { Card, Payment, Transfer } from "./payment";

export function total(payments: Payment[]): number {
  let sum = 0;
  for (const payment of payments) {
    sum += payment.amount();
  }
  return sum;
}

export function receipt(payments: Payment[]): string[] {
  return payments.map((payment) => payment.describe("item"));
}

export function scale(values: number[], step: (value: number) => number): number[] {
  return values.map(step);
}

export function checkout(): string {
  const payments: Payment[] = [new Card(250), new Transfer(100, 5)];
  const lines = receipt(payments);
  const doubled = scale([total(payments)], double);
  return pad(lines.join(", "), 40) + pad(doubled[0]);
}
