"""Payment methods: a base class with two subclasses that override charge."""


class Method:
    """Charges an amount and returns what it took."""

    def charge(self, amount: int) -> int:
        return amount


class Card(Method):
    """Adds a fixed fee."""

    def __init__(self, fee: int) -> None:
        self.fee = fee

    def charge(self, amount: int) -> int:
        return amount + self.fee


class Cash(Method):
    """Rounds down to a multiple of five."""

    def charge(self, amount: int) -> int:
        return amount - amount % 5


def settle(method: Method, amount: int) -> int:
    """Charges amount through method: the call through the base class."""
    return method.charge(amount)
