# CFMD Durable Format V1

## Released authority

`kernel_durability::FORMAT_VERSION == 1` is the current pre-release durable release-candidate discriminator. Compatibility is not frozen until the project actually releases.
All PASS-era format numbers before this point were unreleased implementation tags and are not readable compatibility promises.

The V1 compatibility matrix is intentionally exact:

| Format | Read | Write | Automatic upgrade | Downgrade writer |
|---|---|---|---|---|
| 1 | yes | yes | identity only | no |
| < 1 | no released formats exist | no | no | no |
| > 1 | fail closed | fail closed | no guessed migration | no |

A runtime must reject a newer outer durable format before interpreting its payload. It must not open a newer format read-only by guessing that known sections remain compatible.

## Outer V1 envelope

The released format number is now the common outer authority for both supported durable layouts:

- single-file header/root/generation;
- directory manifest;
- directory checkpoint-file root;
- directory metadata-file root.

These outer owners all write V1. The previous unreleased single-file `3` and directory `3/2/1` tags are deliberately not compatibility branches.

## V1 internal persisted subformats

V1 includes the exact current persisted subformats below. Their numbers are implementation-local, but their current encoding is part of the V1 compatibility obligation because V1 files may contain them:

- WAL frame codec: 1;
- checkpoint semantic revision codec: 7, **current-only decoder**;
- durable physical realization codec: 4, current-only;
- prepared capsule: 1;
- encrypted section envelope: 1;
- wrapped-key slot: 1;
- historical archive/portable-history encodings: current V1 definitions;
- replication authority segment/index/object/locator codecs: current V1 definitions;
- metadata aggregate, mutation and intent-seal tags: current V1 definitions.

Before first release, these encodings may change without preserving old development snapshots; the repository must keep one canonical current grammar and fail closed on obsolete snapshots. At actual release, incompatible changes after the frozen V1 boundary will require a new FORMAT_VERSION and explicit staged upgrade law.

## Pre-release decoder removal

Checkpoint semantic codec versions 1 through 6 are not released formats. V1 reads only the current codec 7. Nested semantic contexts in migration programs and historical realization roots obey the same current-only rule.

This prevents accidental conversion of development history into a permanent compatibility burden.

The repository-wide classification of numeric tags versus actual compatibility paths lives in `docs/status/KERNEL_VERSION_COMPAT_INVENTORY.md`. In particular, PASS568 normalized the sole current checkpoint, realization and persisted canonical equality-key discriminators to `1`. Historical pass-era numbers are not compatibility ancestors. Domain-separation/identity versions remain separate and are not renumbered merely for cosmetic uniformity.

## Upgrade law

V1 has no prior released source version, so `FORMAT_UPGRADE_SOURCES` is empty.

When V2 eventually exists, upgrade must be a staged authority transition:

1. read and verify the complete source authority under its declared released format;
2. construct a complete target-format image without mutating the live source in place;
3. reopen and verify semantic revision, causal/retry/prepared/replication/history/freshness/protection authority;
4. atomically publish/swap the target authority;
5. only then retire the source representation.

A partially rewritten source file is never an upgrade protocol.

## Downgrade / read-only law

V1 exposes no downgrade writer and no implicit read-only compatibility with unknown versions. `FORMAT_DOWNGRADE_TARGETS` is empty.

If a future runtime supports an older released format read-only, that support must appear explicitly in the public compatibility matrix. It may not arise from permissive decoder branches.

## Security law

Format conversion is not a declassification capability. The PASS554 persistence-protection floor and any external freshness authority are preserved by any future upgrade/export/backend transition. Format compatibility cannot be used to write an encrypted/wrapped database into a weaker target.

## Backup / verify / fresh-restore law (PASS559)

A CFMD backup is not an ad-hoc file copy and does not define a second backup byte format. A backup produced by the product API is a quiescent FORMAT V1 single-file realization of one exact `CanonicalPersistenceImage`.

The backup cut preserves the same semantic revision, causal frontier, retry/idempotency authority, unresolved prepared authority, replication authority, retained historical authority and persistence-protection floor. Backup creation publishes no semantic revision and does not replace the live persistence owner.

Backup verification is intentionally stricter than normal live-database recovery. A live database may recover an incomplete non-authoritative WAL tail according to the WAL publication law; a backup artifact must reopen as a clean quiescent authority cut with checkpoint == durable head and no committed live-WAL suffix. Truncation, garbage tail, corrupt root/header/section data, authentication failure or an unsupported format fail closed.

Fresh restore never overwrites an existing target and does not byte-copy the backup into authority. It reopens and strictly verifies the backup, reconstructs its canonical persistence image, then stages/reopens/verifies a fresh target through the same persistence-protection law used by Volatile -> Durable promotion. An encrypted/wrapped backup therefore cannot be restored into a weaker target.

External freshness is not cloneable backup payload. Creating an ordinary backup from a database carrying active external freshness fails closed: two stores must not simultaneously claim one monotonic freshness cut. Disaster-recovery of such a database requires an explicit one-way freshness authority transfer/rebind protocol; it is not ordinary backup copy semantics.

## Deployment / corruption certification matrix (PASS561)

FORMAT V1 is certified against the deployment matrix below. These rows certify the current release-candidate byte language; they become compatibility obligations only when an actual release freezes V1.

| Case | Certified result |
|---|---|
| direct-key source -> plaintext DR target | rejected before target publication; source freshness authority remains unchanged |
| direct-key source -> direct-key DR target | allowed with exact canonical-authority staging; target requires its own correct key and source is fenced by freshness rebind |
| externally wrapped source -> direct-key target | rejected as a protection-authority downgrade |
| externally wrapped source -> different provider identity | rejected before target publication |
| externally wrapped source -> lower provider-key epoch | rejected before target publication |
| externally wrapped source -> same provider identity at equal/higher epoch | allowed; target reopens only with its target wrapping authority |
| failure before freshness rebind CAS | source remains authoritative; staged target remains freshness-sealed and cannot open as an ordinary database |
| rebind CAS committed but response lost | provider state is reconciled; target is accepted only after exact signed-cut verification; source cannot reopen |
| freshness provider process/server restart after rebind | persisted provider journal recovers target store identity and source remains absent |
| committed target subsequently corrupted | target fails closed and source remains fenced; corruption is not authority to roll back the freshness cut |
| torn/corrupt backup or unknown outer FORMAT_VERSION | fails closed; no salvage/fallback to an older root or guessed codec |
| complete wrapped-key header rollback below provider floor | fails closed before database-master-key use |
| encrypted root/section/WAL authentication failure | corruption; never permission to fall back to plaintext or an older authenticated authority |

The critical disaster-recovery invariant is therefore not "one readable file always exists". It is **at most one externally fresh authority exists**. Before rebind, that authority is the source. After a committed rebind, it is the target. If the target is subsequently damaged, the correct state is unavailable/fail-closed rather than resurrection of the old source.

The external freshness provider's durable rebind journal is part of this certification boundary. A provider restart must recover the same target store identity before serving new requests; rebind is not considered a transient in-process fact.
