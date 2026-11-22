import argparse, json, os, re, shutil

here = os.path.dirname(os.path.abspath(__file__))
repo = os.path.normpath(os.path.join(here, "..", ".."))
data = os.path.join(here, "data")

ap = argparse.ArgumentParser(description="Refresh tools/progress/data from the TomBox tree, the title dumps (read-only) and this PC's System32. Needs pefile.")
ap.add_argument("--tombox", default=r"D:\TomBox")
ap.add_argument("--pc", default=os.path.join(os.environ.get("SystemRoot", r"C:\Windows"), "System32"))
ap.add_argument("--config", default=os.path.join(here, "progress.json"))
a = ap.parse_args()
cfg = json.load(open(a.config, encoding="utf-8"))

import pefile


def load(path, dirs):
    pe = pefile.PE(path, fast_load=True)
    pe.parse_data_directories(directories=[pefile.DIRECTORY_ENTRY[d] for d in dirs])
    return pe


def imports(path):
    pe = load(path, ["IMAGE_DIRECTORY_ENTRY_IMPORT", "IMAGE_DIRECTORY_ENTRY_DELAY_IMPORT"])
    mods = []
    for kind, attr in (("import", "DIRECTORY_ENTRY_IMPORT"), ("delay", "DIRECTORY_ENTRY_DELAY_IMPORT")):
        for e in getattr(pe, attr, []):
            mods.append({"module": e.dll.decode(), "kind": kind,
                         "functions": [i.name.decode() if i.name else f"#{i.ordinal}" for i in e.imports]})
    pe.close()
    return mods


def exports(path):
    pe = load(path, ["IMAGE_DIRECTORY_ENTRY_EXPORT"])
    out = []
    if hasattr(pe, "DIRECTORY_ENTRY_EXPORT"):
        for s in pe.DIRECTORY_ENTRY_EXPORT.symbols:
            out.append((s.name.decode(errors="replace") if s.name else None, s.ordinal,
                        s.forwarder.decode(errors="replace") if s.forwarder else None))
    pe.close()
    return out


def version(path):
    try:
        pe = load(path, ["IMAGE_DIRECTORY_ENTRY_RESOURCE"])
        v = pe.VS_FIXEDFILEINFO[0]
        pe.close()
        return "%d.%d.%d.%d" % (v.FileVersionMS >> 16, v.FileVersionMS & 0xFFFF, v.FileVersionLS >> 16, v.FileVersionLS & 0xFFFF)
    except Exception:
        return None


pc_cache = {}


def pc_exports(name):
    key = name.lower()
    if key not in pc_cache:
        p = os.path.join(a.pc, name)
        pc_cache[key] = None
        if os.path.isfile(p):
            try:
                ex = exports(p)
                pc_cache[key] = ({e[0] for e in ex if e[0]}, {f"#{e[1]}": e[0] for e in ex})
            except Exception:
                pass
    return pc_cache[key]


API_HOSTS = ["kernelbase.dll", "kernel32.dll", "ntdll.dll", "ucrtbase.dll", "combase.dll", "user32.dll", "advapi32.dll",
             "sechost.dll", "ole32.dll", "oleaut32.dll", "msvcp_win.dll", "shcore.dll", "rpcrt4.dll", "bcrypt.dll",
             "twinapi.appcore.dll", "wer.dll", "powrprof.dll", "cfgmgr32.dll", "gdi32.dll", "shlwapi.dll", "crypt32.dll"]
api_union = None


def api_set(module):
    global api_union
    p = os.path.join(a.pc, "downlevel", module)
    if os.path.isfile(p):
        ex = exports(p)
        return ({e[0] for e in ex if e[0]}, {f"#{e[1]}": e[0] for e in ex})
    if api_union is None:
        names = set()
        for h in API_HOSTS:
            pc = pc_exports(h)
            if pc:
                names |= pc[0]
        api_union = (names, {})
    return api_union


def pc_has(module, f):
    pc = api_set(module) if module.lower().startswith(("api-ms-", "ext-ms-")) else pc_exports(module)
    if pc is None:
        return 0
    if f.startswith("#"):
        return 2 if f in pc[1] else 0
    return 1 if f in pc[0] else 0


os.makedirs(data, exist_ok=True)
shutil.copyfile(os.path.join(a.tombox, "analysis", "fortnite_exe.json"), os.path.join(data, "fortnite_exe.json"))
shutil.copyfile(os.path.join(a.tombox, "tools", "progress", "mnemonics.txt"), os.path.join(data, "mnemonics.txt"))

src = open(os.path.join(a.tombox, "crates", "tombox-xshader", "src", "rdna2", "mnem.rs"), encoding="utf-8").read()
groups, cur, seen = [], None, set()
for line in src.split("\n"):
    m = re.match(r"pub fn (\w+)\(", line)
    if m:
        cur = {"lookup": "VINTRP"}.get(m.group(1), m.group(1).upper())
    e = re.search(r"Enc::(\w+) =>", line)
    if e:
        cur = {"Vinterp": "VINTRP"}.get(e.group(1), e.group(1).upper())
    if line.lstrip().startswith("//"):
        continue
    for n in re.findall(r'"([a-z][a-z0-9_]*)"', line):
        if n in seen:
            continue
        seen.add(n)
        if not groups or groups[-1][0] != cur:
            groups.append([cur, []])
        groups[-1][1].append(n)
json.dump(groups, open(os.path.join(data, "mnem_groups.json"), "w", encoding="utf-8"), indent=0)

TWINS = {"minuserx.dll": ["user32.dll"], "kernel32legacy.dll": ["kernel32.dll", "kernelbase.dll"],
         "advapi32legacy.dll": ["advapi32.dll"], "minappmodelx.dll": ["twinapi.appcore.dll", "kernel.appcore.dll"]}


def twin_exports(key):
    names, found = set(), []
    for t in TWINS.get(key, ()):
        pc = pc_exports(t)
        if pc:
            names |= pc[0]
            found.append(t)
    return names, found


CONSOLE_HOSTED = {"minuserx.dll", "advapi32legacy.dll", "kernel32legacy.dll", "minappmodelx.dll"}
hosts_file = os.path.join(a.tombox, "crates", "compat", "compat-common", "data", "console_apiset_hosts.txt")
apiset_hosts = {}
for line in open(hosts_file, encoding="utf-8") if os.path.isfile(hosts_file) else ():
    f = line.split()
    if len(f) == 4 and f[0].isdigit():
        apiset_hosts.setdefault((f[1].lower(), f[2]), []).append((int(f[0]), f[3].lower()))


def apiset_host(module, f, build):
    low = module.lower()
    rows = apiset_hosts.get((low if low.endswith(".dll") else low + ".dll", f))
    if not rows or build is None:
        return None
    le = [r for r in rows if r[0] <= build]
    return max(le)[1] if le else min(rows)[1]


def tpath(t, key):
    base = repo if t.get("base") == "repo" else a.tombox
    return os.path.join(base, *t[key].split("/"))


def build_num(v):
    try:
        return int(str(v).split(".")[2])
    except (IndexError, ValueError, TypeError):
        return None


titles, console, skipped, cons_ord = {}, {}, [], {}
for t in cfg["titles_analysed"]:
    bdir = tpath(t, "binaries")
    exe = os.path.join(bdir, t["exe"])
    if not os.path.isfile(exe) or ("exe_size" in t and os.path.getsize(exe) != t["exe_size"]):
        skipped.append(t["name"])
        continue
    xvd = tpath(t, "xvd") if t.get("xvd") else None
    os_build = version(os.path.join(xvd, "System32", "ntdll.dll")) if xvd else None
    tord = cons_ord.setdefault(t["name"], {})
    for sub in ("System32", "System32/forwarders") if xvd else ():
        d = os.path.join(xvd, *sub.split("/"))
        if not os.path.isdir(d):
            continue
        for name in sorted(os.listdir(d), key=str.lower):
            p = os.path.join(d, name)
            if not os.path.isfile(p) or not name.lower().endswith(".dll"):
                continue
            ex = exports(p)
            if not ex:
                continue
            key = name.lower()
            tord.setdefault(key, {f"#{o}": n for n, o, fwd in ex if n})
            pc = pc_exports(name)
            lib = console.setdefault(key, {"name": name, "dir": sub, "pc_dll": pc is not None, "exports": {}, "versions": {}})
            lib["versions"][t["name"]] = version(p)
            for n, o, fwd in ex:
                k = n or f"#{o}"
                row = lib["exports"].setdefault(k, [0 if pc is None else (1 if n in pc[0] else 0) if n else (2 if k in pc[1] else 0), fwd, []])
                row[2].append(t["name"])
    bnum = build_num(os_build) or build_num(t.get("os_build_hint"))
    bins, files = {}, {}
    for sub in t.get("binary_dirs", [""]):
        d = os.path.join(bdir, *filter(None, sub.split("/")))
        if not os.path.isdir(d):
            continue
        shipped = {n.lower() for n in os.listdir(d)}
        for name in sorted(os.listdir(d), key=str.lower):
            p = os.path.join(d, name)
            if os.path.isfile(p) and name.lower().endswith((".exe", ".dll")):
                mods = imports(p)
                for m in mods:
                    low = m["module"].lower()
                    m["pc"] = [pc_has(m["module"], f) for f in m["functions"]]
                    if low.startswith(("api-ms-", "ext-ms-")):
                        hs = [apiset_host(low, f, bnum) for f in m["functions"]]
                        hs = [h if h in CONSOLE_HOSTED else None for h in hs]
                        if any(hs):
                            m["hosts"] = hs
                            m["pc"] = [0 if h else pf for h, pf in zip(hs, m["pc"])]
                    elif low not in shipped and low in tord:
                        nm = [tord[low].get(f) if f.startswith("#") else None for f in m["functions"]]
                        if any(nm):
                            m["names"] = nm
                bins[f"{sub}/{name}" if sub else name] = mods
                files[f"{sub}/{name}" if sub else name] = p
    titles[t["name"]] = {"exe": t["exe"], "binaries": bins, "os_build": os_build, "build": bnum}
    if os_build is None and t.get("os_build_hint"):
        titles[t["name"]]["os_build_hint"] = t["os_build_hint"]
    titles[t["name"]]["_files"] = files

dyn_names = {}
for key, lib in console.items():
    names = {k for k, v in lib["exports"].items() if v[0] == 0 and not k.startswith("#")}
    if names:
        dyn_names[key] = (lib["name"], names)
STR_RE = re.compile(rb"(?<![\x20-\x7e])[\x21-\x7e]{2,255}(?=\x00)")
for title, t in titles.items():
    dyn = {}
    for b, p in t.pop("_files").items():
        raw = open(p, "rb").read()
        low = raw.lower()
        strs = None
        static = {m["module"].lower() for m in t["binaries"][b]}
        for key, (name, names) in dyn_names.items():
            if key in static or (key.encode() not in low and key.encode("utf-16-le") not in low):
                continue
            if strs is None:
                strs = set(STR_RE.findall(raw))
            hit = sorted(n for n in names if n.encode() in strs)
            if hit:
                dyn.setdefault(b, {})[name] = hit
        del raw, low, strs
    if dyn:
        t["dynamic"] = dyn

out = {}
for key, lib in console.items():
    out[lib["name"]] = {"dir": lib["dir"], "pc_dll": lib["pc_dll"], "versions": lib["versions"],
                        "exports": [[k, v[0], v[1], v[2]] for k, v in lib["exports"].items()]}
    if not lib["pc_dll"] and key in TWINS:
        tn, found = twin_exports(key)
        out[lib["name"]]["pc_twin"] = found
        out[lib["name"]]["twin_has"] = sorted(k for k in lib["exports"] if k in tn)
json.dump(out, open(os.path.join(data, "console_exports.json"), "w", encoding="utf-8"), separators=(",", ":"))
json.dump(titles, open(os.path.join(data, "title_imports.json"), "w", encoding="utf-8"), indent=1)


def read_table(path):
    funcs, notes = {}, {}
    for line in open(path, encoding="utf-8"):
        body, _, note = line.strip().partition(" | ")
        f = body.split()
        if len(f) < 3:
            continue
        funcs[f[1]] = f[2]
        if note:
            notes[f[1]] = note.strip()
    return funcs, notes


cdir = os.path.join(a.tombox, "crates", "compat")


def lib_of(crate):
    toml = os.path.join(cdir, crate, "Cargo.toml")
    if not os.path.isfile(toml):
        return None
    lib = re.search(r'\[lib\][^\[]*?name\s*=\s*"([^"]+)"', open(toml, encoding="utf-8").read())
    return lib.group(1) if lib else crate


def built_exports(libname):
    p = os.path.join(a.tombox, "target", "release", libname + ".dll")
    if not os.path.isfile(p):
        return None
    ex = exports(p)
    return {n for n, o, f in ex if n} | {f"#{o}" for n, o, f in ex}


emu = {}
api_builds = {b for v in apiset_hosts.values() for b, _ in v}
for crate in sorted(os.listdir(cdir), key=str.lower):
    base = next((os.path.join(cdir, crate, n) for n in ("exports.txt", "map_exports.txt") if os.path.isfile(os.path.join(cdir, crate, n))), None)
    if base is None:
        continue
    libname = lib_of(crate) or crate
    dll = libname + ".dll"
    dll = next((n for n in out if n.lower() == dll.lower()), dll)
    funcs, notes = read_table(base)
    entry = {"file": f"crates/compat/{crate}/src/lib.rs", "table": f"crates/compat/{crate}/{os.path.basename(base)}", "functions": funcs, "notes": notes}
    built = built_exports(libname)
    if built is not None:
        entry["not_in_built_dll"] = sorted(n for n, s in funcs.items() if s != "missing" and n not in built)
    builds = {}
    for n in sorted(os.listdir(os.path.join(cdir, crate))):
        m = re.fullmatch(r"exports_(\d+)\.txt", n)
        if m:
            bf, bn = read_table(os.path.join(cdir, crate, n))
            bt = {"table": f"crates/compat/{crate}/{n}", "lib": f"{libname}_{m.group(1)}", "functions": bf, "notes": bn}
            bb = built_exports(bt["lib"])
            if bb is not None:
                bt["not_in_built_dll"] = sorted(k for k, s in bf.items() if s != "missing" and k not in bb)
            builds[m.group(1)] = bt
    if builds:
        entry["builds"] = builds
        entry["known_builds"] = sorted(api_builds | {int(b) for b in builds})
    emu[dll] = entry
cfg_checks = {}
for dll, v in cfg.get("emulator", {}).items():
    m = re.match(r"crates/compat/([^/]+)/", v.get("file", ""))
    libname = lib_of(m.group(1)) if m else None
    built = built_exports(libname) if libname else None
    if built is not None:
        cfg_checks[dll] = {"built_dll": f"target/release/{libname}.dll",
                           "not_in_built_dll": sorted(n for n, s in v.get("functions", {}).items() if s != "missing" and n not in built)}
json.dump(emu, open(os.path.join(data, "emulator_exports.json"), "w", encoding="utf-8"), indent=1)
json.dump(cfg_checks, open(os.path.join(data, "emulator_cfg_checks.json"), "w", encoding="utf-8"), indent=1)

EVIDENCE = ["chatprobe.json", "media.json", "gameinput_ext.json", "oshost.json"]
edir, odir = os.path.join(a.tombox, "analysis", "progress_evidence"), os.path.join(data, "evidence")
os.makedirs(odir, exist_ok=True)
copied = []
for name in EVIDENCE:
    p = os.path.join(edir, name)
    if os.path.isfile(p):
        json.load(open(p, encoding="utf-8"))
        shutil.copyfile(p, os.path.join(odir, name))
        copied.append(name)
    elif os.path.isfile(os.path.join(odir, name)):
        os.remove(os.path.join(odir, name))

ords = {}
for dll in ("XAudio2_9.dll", "GameInput.dll"):
    p = os.path.join(a.pc, dll)
    if os.path.isfile(p):
        ords[dll] = {f"#{o}": n for n, o, f in exports(p) if n}
json.dump(ords, open(os.path.join(data, "pc_ordinals.json"), "w", encoding="utf-8"), indent=1)

print("titles", list(titles), "skipped", skipped, "console libraries", len(out),
      "console exports", sum(len(v["exports"]) for v in out.values()),
      "os builds", {k: v["os_build"] for k, v in titles.items()},
      "emulator export tables", {k: len(v["functions"]) for k, v in emu.items()},
      "twins", {k: (v.get("pc_twin"), len(v.get("twin_has", []))) for k, v in out.items() if "pc_twin" in v},
      "evidence", copied)
