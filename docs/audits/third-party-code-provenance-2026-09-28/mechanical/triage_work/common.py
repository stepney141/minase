import re,csv,sys,json,os
sys.path.insert(0,os.path.dirname(os.path.abspath(__file__)))
from loc import parse
from inv import INV
csv.field_size_limit(10**9)
VENDOR=re.compile(r'(nlohmann|third_party|3rdparty|thirdparty|/external/|^external/|vendor|gaviota|zlib|packages/|node_modules|idna|json\.hpp|catch\.hpp|doctest|/lib/(fmt|spdlog)|incbin|pybind11|googletest|gtest|/deps/|syzygy/tbprobe|Fathom|pyrrhic|/tb/|stb_|miniz|lz4|zstd|/msgpack|cxxopts|popl|argparse\.h|dist-packages|site-packages|\.min\.js)',re.I)
MTEST=re.compile(r'(^|/)tests?(/|\.rs$)|_tests?\.(rs|py)$|(^|/)test_[^/]*\.py$|/tests/|validation-fixtures|testdata|fixtures?/')
CTEST=re.compile(r'(^|/)(tests?|testing|unittest|unit_tests?)(/|\.)|_tests?\.|(^|/)test_|Test\.cpp$|_spec\.|/spec/',re.I)
SRC={}
def src_lines(rid):
    if rid not in SRC:
        try: SRC[rid]=open(INV[rid]['source'],errors='replace').read().split('\n')
        except Exception: SRC[rid]=[]
    return SRC[rid]
TESTRANGES={}
MODOPEN=re.compile(r'^\s*(pub(\([^)]*\))?\s+)?mod\s+\w+\s*\{')
def test_ranges(rid):
    """Line ranges of inline `#[cfg(test)] mod X { ... }` blocks (1-based, inclusive)."""
    if rid in TESTRANGES: return TESTRANGES[rid]
    L=src_lines(rid); R=[]
    i=0
    while i<len(L):
        if L[i].strip().startswith('#[cfg(test)]'):
            j=i+1
            while j<len(L) and (not L[j].strip() or L[j].strip().startswith('#[') or L[j].strip().startswith('//')): j+=1
            if j<len(L) and MODOPEN.match(L[j]):
                depth=0;k=j
                while k<len(L):
                    depth+=L[k].count('{')-L[k].count('}')
                    if depth<=0 and k>j: break
                    k+=1
                R.append((i+1,k+1)); i=k
        i+=1
    TESTRANGES[rid]=R; return R
def rust_test_line(rid,line):
    """True if line is inside an inline #[cfg(test)] mod block or the fn has #[test]."""
    if any(a<=line<=b for a,b in test_ranges(rid)): return True
    L=src_lines(rid)
    for j in range(line-2,max(line-6,-1),-1):
        if 0<=j<len(L) and L[j].strip().startswith('#[test]'): return True
    return False
def mtest(rid,path,line):
    if MTEST.search(path): return True
    if path.endswith('.rs') and rust_test_line(rid,line): return True
    return False
def strip_repo(c): return c['repo']+':'+c['path']
