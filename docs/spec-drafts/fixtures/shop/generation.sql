-- DRAFT view fixture "shop" (P2.1, P7.1). On approval this moves to
-- tests/fixtures/views/shop/generation.sql.
--
-- One complete generation of a small Rust project, written against
-- codetags-model's SCHEMA_SQL (schema version 1). The step
-- `Given the view fixture "shop"` creates the schema, runs this file, and
-- completes the generation as generation 1. See docs/spec-drafts/steps-proposed.md
-- for the facets the drafts assume P2.4 derives from these rows.
--
-- Canonical names (PLAN.md §2.7) the SCIP symbols below map to:
--
--   api.place_order               function  src/api/routes.rs       4-12
--   billing.Charge                struct    src/billing/charge.rs   3-6
--   billing.Charge<T>.apply       method    src/billing/charge.rs   12-30
--   billing.Order                 struct    src/billing/order.rs    3-7
--   billing.order                 function  src/billing/order.rs    9-12
--   notify.Mailer.send            method    src/notify/mail.rs      8-15
--   ordering.OrderRepo            trait     src/ordering/repo.rs    5-9
--   ordering.OrderRepo.save       method    src/ordering/repo.rs    8-8
--   ordering.PgOrderRepo.save     method    src/ordering/repo.rs    20-34
--   ordering.OrderService         struct    src/ordering/service.rs 10-14
--   ordering.OrderService.place   method    src/ordering/service.rs 40-70
--
-- Call sites:
--
--   site  caller                       line  receiver     -> target                    dispatch  targets (method)
--   4412  ordering.OrderService.place  58    self.repo    -> ordering.OrderRepo.save   virtual   OrderRepo.save (declared), PgOrderRepo.save (cha)
--   4413  ordering.OrderService.place  61    self.charge  -> billing.Charge<T>.apply   static    declared
--   4414  ordering.OrderService.place  63    self.charge  -> billing.Charge<T>.apply   static    declared
--   4415  ordering.OrderService.place  66    self.mailer  -> notify.Mailer.send        static    declared
--   4416  api.place_order              9     service      -> ordering.OrderService.place static   declared
--   4417  billing.Charge<T>.apply      20    (none)       -> billing.order             static    declared
--   4418  ordering.PgOrderRepo.save    30    self.pool    -> (unresolved)              dynamic   none
--
-- Module edges (call sites whose caller and some target are in different
-- modules): api -> ordering 1, ordering -> billing 2, ordering -> notify 1.

INSERT INTO run (run_id, provider, provider_version, args, started_at, finished_at, status, source_tree)
VALUES (1, 'rust-analyzer-scip', '1.96.1', ['scip', '.'],
        TIMESTAMPTZ '2026-09-30 11:58:00+00', TIMESTAMPTZ '2026-09-30 12:00:00+00',
        'succeeded', 'a1b2c3d4e5f60718293a4b5c6d7e8f9012345678');

INSERT INTO run_file (run_id, path, edge_count) VALUES
  (1, 'src/api/routes.rs',       1),
  (1, 'src/billing/charge.rs',   1),
  (1, 'src/billing/order.rs',    0),
  (1, 'src/notify/mail.rs',      0),
  (1, 'src/ordering/repo.rs',    0),
  (1, 'src/ordering/service.rs', 5);

INSERT INTO file (path, language, content_hash, size) VALUES
  ('src/api/routes.rs',       'rust', 'sha256:01', 412),
  ('src/billing/charge.rs',   'rust', 'sha256:02', 903),
  ('src/billing/order.rs',    'rust', 'sha256:03', 288),
  ('src/notify/mail.rs',      'rust', 'sha256:04', 377),
  ('src/ordering/repo.rs',    'rust', 'sha256:05', 1104),
  ('src/ordering/service.rs', 'rust', 'sha256:06', 2210);

INSERT INTO symbol (id, lang, kind, file, start_line, end_line, signature, module) VALUES
  ('rust-analyzer cargo shop 0.1.0 api/place_order().',             'rust', 'function', 'src/api/routes.rs',        4, 12, 'pub fn place_order(req: Request) -> Response',          'api'),
  ('rust-analyzer cargo shop 0.1.0 billing/Charge#',                'rust', 'struct',   'src/billing/charge.rs',    3,  6, 'pub struct Charge<T>',                                  'billing'),
  ('rust-analyzer cargo shop 0.1.0 billing/`Charge<T>`#apply().',   'rust', 'method',   'src/billing/charge.rs',   12, 30, 'pub fn apply(&self, amount: Money) -> Result<Receipt>', 'billing'),
  ('rust-analyzer cargo shop 0.1.0 billing/Order#',                 'rust', 'struct',   'src/billing/order.rs',     3,  7, 'pub struct Order',                                      'billing'),
  ('rust-analyzer cargo shop 0.1.0 billing/order().',               'rust', 'function', 'src/billing/order.rs',     9, 12, 'pub fn order(id: OrderId) -> Order',                    'billing'),
  ('rust-analyzer cargo shop 0.1.0 notify/Mailer#send().',          'rust', 'method',   'src/notify/mail.rs',       8, 15, 'pub fn send(&self, to: &str, body: &str)',              'notify'),
  ('rust-analyzer cargo shop 0.1.0 ordering/OrderRepo#',            'rust', 'trait',    'src/ordering/repo.rs',     5,  9, 'pub trait OrderRepo',                                   'ordering'),
  ('rust-analyzer cargo shop 0.1.0 ordering/OrderRepo#save().',     'rust', 'method',   'src/ordering/repo.rs',     8,  8, 'fn save(&self, order: &Order) -> Result<OrderId>',      'ordering'),
  ('rust-analyzer cargo shop 0.1.0 ordering/PgOrderRepo#save().',   'rust', 'method',   'src/ordering/repo.rs',    20, 34, 'fn save(&self, order: &Order) -> Result<OrderId>',      'ordering'),
  ('rust-analyzer cargo shop 0.1.0 ordering/OrderService#',         'rust', 'struct',   'src/ordering/service.rs', 10, 14, 'pub struct OrderService',                               'ordering'),
  ('rust-analyzer cargo shop 0.1.0 ordering/OrderService#place().', 'rust', 'method',   'src/ordering/service.rs', 40, 70, 'pub fn place(&self, order: Order) -> Result<OrderId>',  'ordering');

INSERT INTO call_site (site_id, caller, file, line, ordinal, receiver_text, declared_target, dispatch, source) VALUES
  (4412, 'rust-analyzer cargo shop 0.1.0 ordering/OrderService#place().',   'src/ordering/service.rs', 58, 1, 'self.repo',   'rust-analyzer cargo shop 0.1.0 ordering/OrderRepo#save().',     'virtual', 'scip@a1b2c3d'),
  (4413, 'rust-analyzer cargo shop 0.1.0 ordering/OrderService#place().',   'src/ordering/service.rs', 61, 2, 'self.charge', 'rust-analyzer cargo shop 0.1.0 billing/`Charge<T>`#apply().',   'static',  'scip@a1b2c3d'),
  (4414, 'rust-analyzer cargo shop 0.1.0 ordering/OrderService#place().',   'src/ordering/service.rs', 63, 3, 'self.charge', 'rust-analyzer cargo shop 0.1.0 billing/`Charge<T>`#apply().',   'static',  'scip@a1b2c3d'),
  (4415, 'rust-analyzer cargo shop 0.1.0 ordering/OrderService#place().',   'src/ordering/service.rs', 66, 4, 'self.mailer', 'rust-analyzer cargo shop 0.1.0 notify/Mailer#send().',          'static',  'scip@a1b2c3d'),
  (4416, 'rust-analyzer cargo shop 0.1.0 api/place_order().',               'src/api/routes.rs',        9, 1, 'service',     'rust-analyzer cargo shop 0.1.0 ordering/OrderService#place().', 'static',  'scip@a1b2c3d'),
  (4417, 'rust-analyzer cargo shop 0.1.0 billing/`Charge<T>`#apply().',     'src/billing/charge.rs',   20, 1, NULL,          'rust-analyzer cargo shop 0.1.0 billing/order().',               'static',  'scip@a1b2c3d'),
  (4418, 'rust-analyzer cargo shop 0.1.0 ordering/PgOrderRepo#save().',     'src/ordering/repo.rs',    30, 1, 'self.pool',   NULL,                                                             'dynamic', 'scip@a1b2c3d');

INSERT INTO call_target (site_id, target, method) VALUES
  (4412, 'rust-analyzer cargo shop 0.1.0 ordering/OrderRepo#save().',     'declared'),
  (4412, 'rust-analyzer cargo shop 0.1.0 ordering/PgOrderRepo#save().',   'cha'),
  (4413, 'rust-analyzer cargo shop 0.1.0 billing/`Charge<T>`#apply().',   'declared'),
  (4414, 'rust-analyzer cargo shop 0.1.0 billing/`Charge<T>`#apply().',   'declared'),
  (4415, 'rust-analyzer cargo shop 0.1.0 notify/Mailer#send().',          'declared'),
  (4416, 'rust-analyzer cargo shop 0.1.0 ordering/OrderService#place().', 'declared'),
  (4417, 'rust-analyzer cargo shop 0.1.0 billing/order().',               'declared');
