# Thread pagination

Thread listing examines at most 512 candidate roots per request. Visibility is
still checked independently for each root. A page can therefore have an empty
`chunk` and a `next_batch`; clients should follow that token to continue scanning.
This preserves access to older visible history without scanning an entire room
for every request from a nonmember.

New `t2` tokens encrypt the scan position with AES-256-GCM and a fresh random
nonce. They bind the requesting user, room, include mode, ordering and protocol
version, expire after one hour, and cannot expose a private root ID. The key is
derived from the server's private signing seed with domain-separated HKDF-SHA256.
Use the same persisted signing key across instances to allow continuation after
load balancing or restart. An ephemeral signing key, or signing-key rotation,
invalidates existing `t2` tokens; clients must restart pagination in that case.

Older `t1` visible-anchor tokens remain accepted. Their root must still be
visible to the requesting user, and their activity position must identify the
root or a real reply to it. Newer activity does not invalidate an older anchor.
