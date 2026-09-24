#!/usr/bin/env python3
from pathlib import Path
import re
import sys

root = Path(__file__).resolve().parents[2]
store = (root / 'crates/kernel-durability/src/store.rs').read_text()
model = (root / 'crates/kernel-durability/src/store/publication_model.rs').read_text()
lean = (root / 'formal/lean/CFMD/Publication.lean').read_text()

expected = [
    'AfterCheckpointSync',
    'AfterWalSync',
    'AfterMetadataSync',
    'AfterPrerequisiteDirectorySync',
    'AfterPendingManifestSync',
    'AfterManifestRename',
    'AfterManifestDirectorySync',
    'BeforeCompactionRemove',
    'AfterCompactionRemove',
    'AfterCompactionDirectorySync',
]

m = re.search(r'enum StoreFaultPoint\s*\{(?P<body>.*?)\n\}', store, re.S)
if not m:
    raise SystemExit('StoreFaultPoint enum not found')
actual = re.findall(r'^\s*([A-Za-z][A-Za-z0-9_]*)\s*,\s*$', m.group('body'), re.M)
if actual != expected:
    raise SystemExit(f'StoreFaultPoint mismatch: {actual!r}')

for name in expected:
    if f'StoreFaultPoint::{name}' not in model:
        raise SystemExit(f'Rust model refinement missing {name}')

lean_names = [name[0].lower() + name[1:] for name in expected]
for name in lean_names:
    if f'| {name}' not in lean:
        raise SystemExit(f'Lean fault-point mirror missing {name}')

# Production protocol ordering witnesses.  These are intentionally lexical
# because they validate that the hooked source still exposes the same cuts.
ordered_needles = [
    'hook.hit(StoreFaultPoint::AfterCheckpointSync)?;',
    'hook.hit(StoreFaultPoint::AfterWalSync)?;',
    'hook.hit(StoreFaultPoint::AfterMetadataSync)?;',
    'hook.hit(StoreFaultPoint::AfterPrerequisiteDirectorySync)?;',
]
positions = [store.find(n) for n in ordered_needles]
if any(p < 0 for p in positions) or positions != sorted(positions):
    raise SystemExit(f'prerequisite fault-point order changed: {positions}')

publish_start = store.find('fn publish_manifest_with_hook(')
publish_end = store.find('\nfn encode_manifest', publish_start)
publish = store[publish_start:publish_end]
publish_needles = [
    'file.sync_all()?;',
    'hook.hit(StoreFaultPoint::AfterPendingManifestSync)?;',
    '*authority_uncertain = true;',
    'fs::rename(&pending_path, &final_path)?;',
    'hook.hit(StoreFaultPoint::AfterManifestRename)?;',
    'sync_directory(directory)?;',
    'hook.hit(StoreFaultPoint::AfterManifestDirectorySync)?;',
]
pos = [publish.find(n) for n in publish_needles]
if any(p < 0 for p in pos) or pos != sorted(pos):
    raise SystemExit(f'manifest publication refinement changed: {pos}')

compact_start = store.find('fn compact_obsolete_generations_with_hook(')
compact_end = store.find('\nfn bind_prepare_descriptor', compact_start)
compact = store[compact_start:compact_end]
compact_needles = [
    'hook.hit(StoreFaultPoint::BeforeCompactionRemove)?;',
    'fs::remove_file(entry.path())?;',
    'hook.hit(StoreFaultPoint::AfterCompactionRemove)?;',
    'sync_directory(&self.directory)?;',
    'hook.hit(StoreFaultPoint::AfterCompactionDirectorySync)?;',
]
pos = [compact.find(n) for n in compact_needles]
if any(p < 0 for p in pos) or pos != sorted(pos):
    raise SystemExit(f'GC refinement changed: {pos}')

# Streaming publication must close every extra immutable prerequisite before it
# enters the same manifest publisher proved above.
stream_start = store.find('pub fn begin_streaming_checkpoint(')
stream_end = store.find('pub fn write_streaming_checkpoint_chunks(', stream_start)
start_stream = store[stream_start:stream_end]
for needle in [
    'write_prepared_cut_capsule(',
    'write_metadata_file(',
    'FileRevisionWal::create_at_lsn(',
    'shadow_wal.durability_barrier()?;',
    'sync_directory(&self.directory)?;',
]:
    if needle not in start_stream:
        raise SystemExit(f'streaming start prerequisite missing: {needle}')

chunks_start = store.find('pub fn write_streaming_checkpoint_chunks(')
chunks_end = store.find('pub fn finalize_streaming_checkpoint(', chunks_start)
chunks = store[chunks_start:chunks_end]
for needle in ['file.sync_all()?;', 'write_chunked_checkpoint_root(', 'sync_directory(&self.directory)?;']:
    if needle not in chunks:
        raise SystemExit(f'streaming chunk durability step missing: {needle}')

final_start = store.find('pub fn finalize_streaming_checkpoint(')
final_end = store.find('pub fn abort_streaming_checkpoint', final_start)
if final_end < 0:
    final_end = store.find('pub fn ', final_start + 40)
finalize = store[final_start:final_end]
final_needles = [
    '.durability_barrier()',
    'self.barrier_streaming_shadow();',
    'read_checkpoint_generation(',
    'prepared_capsule_path(',
    'sync_directory(&self.directory)?;',
    'publish_manifest_with_hook(',
]
pos = [finalize.find(n) for n in final_needles]
if any(p < 0 for p in pos) or pos != sorted(pos):
    raise SystemExit(f'streaming finalize refinement changed: {pos}')

# Generation creation remains monotone by construction through next_generation.
if store.count('let generation = next_generation(&self.directory)?;') < 2:
    raise SystemExit('generation monotonicity binding changed')

# Immutable generation artifacts must be created as new files rather than
# opened for in-place overwrite.
for fn_name in [
    'fn write_checkpoint_file(',
    'fn write_metadata_file(',
    'fn write_prepared_cut_capsule(',
]:
    start = store.find(fn_name)
    if start < 0:
        raise SystemExit(f'immutable writer missing: {fn_name}')
    end = store.find('\nfn ', start + len(fn_name))
    body = store[start:end if end >= 0 else len(store)]
    if '.create_new(true)' not in body:
        raise SystemExit(f'immutable writer no longer uses create_new: {fn_name}')

print('P18 refinement check: PASS')
print('fault points:', len(expected))
