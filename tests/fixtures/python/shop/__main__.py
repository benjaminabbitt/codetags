"""Runs the shop: python -m shop.

main refers to Printer before Printer's definition: a plain class, to show
that the missing SymbolInformation of sourcegraph/scip-python#223 is not
about dataclasses.
"""

from shop.pay.methods import Card, Cash
from shop.worker import Worker, notify


def main() -> None:
    for method in (Card(2), Cash()):
        for receipt in Worker(method).run([10, 12]):
            notify(Printer(), receipt)


class Printer:
    """A sink for notify."""

    def send(self, text: str) -> None:
        print(text)


if __name__ == "__main__":
    main()
