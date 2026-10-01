// The entry point: idiomatic TypeScript, then the JavaScript-style module.

import { checkout } from "./checkout";
import { dispatch } from "./legacy";
import { Card } from "./payment";

export function main(): string {
  const summary = checkout();
  const routed = dispatch("card", new Card(7));
  return summary + routed;
}

main();
