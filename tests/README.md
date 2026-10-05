# Complement

## What's that?

Have a look at [its repository](https://github.com/matrix-org/complement).

## How do I use it with Palpo?

The script at [`../complement`](../bin/complement) has automation for this.
It takes a few command line arguments, you can read the script to find out what
those are.


```bash
bash ./tests/complement.sh ../complement  __test.result.log  __test.result.jsonl
```
## Appservice delivery retry

`appservice_delivery_retry.py` drives a disposable real Palpo homeserver and its
real database. Its local appservice receiver first returns 503, then 200. The
check requires the same transaction ID and stable event content on retry. The
only ignored event field is `unsigned.age`, which advances between deliveries.

```sh
python3 tests/appservice_delivery_retry.py \
  --origin http://127.0.0.1:24682 \
  --accounts /private/adr0011/accounts.json \
  --binary target/debug/palpo \
  --output target/appservice-retry-evidence
```

Use owner-private JSON containing `admin` and `manager` entries with `user_id`
and `access_token` for the isolated `rinx-adr0011.test` server. The test creates
an isolated bot and room; it unregisters its temporary appservice afterward.
The report records the executable SHA-256, transaction IDs, event IDs and event
digests, and excludes credentials. Keep the tested binary unchanged during the
run. This check does not substitute for encrypted agent or Rinx UI acceptance.
