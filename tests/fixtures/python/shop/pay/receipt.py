"""Dataclasses, to check scip-python's dataclass symbol information.

Receipt is referenced only after its definition. Refund is referenced in
refund_all before its definition, the trigger sourcegraph/scip-python#223
describes for a missing SymbolInformation.
"""

from dataclasses import dataclass


@dataclass
class Receipt:
    """What a payment took."""

    method: str
    amount: int

    def describe(self) -> str:
        return f"{self.method}: {self.amount}"


def refund_all(receipts):
    return [Refund(receipt) for receipt in receipts]


@dataclass(frozen=True)
class Refund:
    """A receipt paid back."""

    receipt: Receipt

    def describe(self) -> str:
        return "refund of " + self.receipt.describe()
