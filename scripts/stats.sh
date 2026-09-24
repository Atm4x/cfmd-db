#!/usr/bin/env bash
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
python3 - <<'PY'
from pathlib import Path
import re
root=Path('.')
all_rs=list((root/'crates').glob('**/*.rs'))
src_rs=list((root/'crates').glob('*/src/**/*.rs'))
lean=list((root/'formal').glob('**/*.lean'))
py=list((root/'formal').glob('**/*.py'))
current_docs=[p for base in [root/'README.md',root/'SPEC.md',root/'docs'/'architecture',root/'docs'/'status',root/'docs'/'api',root/'docs'/'reports'/'current'] for p in ([base] if base.is_file() else base.glob('**/*.md'))]

def lines(files):
    return sum(len(p.read_text(encoding='utf-8',errors='ignore').splitlines()) for p in files)
def nonblank(files):
    return sum(sum(1 for x in p.read_text(encoding='utf-8',errors='ignore').splitlines() if x.strip()) for p in files)
crates=len(list((root/'crates').glob('*/Cargo.toml')))
test_attrs=sum(len(re.findall(r'#\[(?:tokio::)?test\]',p.read_text(encoding='utf-8',errors='ignore'))) for p in all_rs)
print(f'workspace_crates={crates}')
print(f'rust_files_all={len(all_rs)}')
print(f'rust_lines_all={lines(all_rs)}')
print(f'rust_nonblank_lines_all={nonblank(all_rs)}')
print(f'rust_source_files={len(src_rs)}')
print(f'rust_source_lines={lines(src_rs)}')
print(f'lean_files={len(lean)}')
print(f'lean_lines={lines(lean)}')
print(f'formal_python_lines={lines(py)}')
print(f'rust_test_attributes={test_attrs}')
print(f'current_project_docs_lines={lines(current_docs)}')
print(f'historical_archive_files={sum(1 for base in [root/"docs/history", root/"docs/reports/archive"] for p in base.rglob("*") if p.is_file())}')
print(f'evidence_files={sum(1 for p in (root/"artifacts").rglob("*") if p.is_file())}')
PY
