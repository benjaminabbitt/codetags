// An interface with two implementations, called through the interface.

export interface Payment {
  amount(): number;
  describe(label: string): string;
}

export class Card implements Payment {
  constructor(private readonly cents: number) {}

  amount(): number {
    return this.cents;
  }

  describe(label: string): string {
    return label + ": card " + this.amount();
  }
}

export class Transfer implements Payment {
  constructor(private readonly cents: number, private readonly fee: number) {}

  amount(): number {
    return this.cents - this.fee;
  }

  describe(label: string): string {
    return label + ": transfer " + this.amount();
  }
}
