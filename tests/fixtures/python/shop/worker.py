"""A method passed as a callback, and a duck-typed call Pyright cannot resolve."""

from concurrent.futures import ThreadPoolExecutor

from shop.pay.methods import Method, settle
from shop.pay.receipt import Receipt


class Worker:
    """Settles payments on a thread pool."""

    def __init__(self, method: Method) -> None:
        self.method = method

    def process(self, amount: int) -> Receipt:
        return Receipt(type(self.method).__name__, settle(self.method, amount))

    def run(self, amounts: list[int]) -> list[Receipt]:
        with ThreadPoolExecutor() as executor:
            futures = [executor.submit(self.process, amount) for amount in amounts]
            return [future.result() for future in futures]


def notify(sink, receipt):
    """sink is duck-typed: nothing says what it is, so send is unresolved."""
    sink.send(receipt.describe())
