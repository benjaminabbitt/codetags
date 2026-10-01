// JavaScript-style code: no types, handlers built and chosen dynamically.

function makeHandlers() {
  var handlers = {};
  handlers.card = function (payment) {
    return payment.amount();
  };
  handlers.transfer = function (payment) {
    return payment.amount() * 2;
  };
  return handlers;
}

function dispatch(kind, payment) {
  var handlers = makeHandlers();
  return handlers[kind](payment);
}

module.exports = { dispatch: dispatch, makeHandlers: makeHandlers };
