# From EZ-Language to GluCore — What Changed and Why

> A technical retrospective on a project I started as a Minecraft mod problem,
> which became an accidental programming language, then became something I couldn't
> have predicted at the start: a manifest-driven polyglot runtime.

---

## The Origin Problem (Unchanged)

Both of these projects start from the same place. I had a Minecraft mod (Java) that needed C++ or Rust
for heavy computation. Every existing option — Bazel, FFI, gRPC, WebAssembly,
transpilers — felt like solving the tooling problem instead of the actual
problem. My vision was simple:

> A coordinator between languages. Describe what you want. The system handles the glue.

That vision never changed. Everything else did.

---

## What EZ-Language Was

EZ-Language was a **custom programming language** I built with:

- A hand-rolled lexer, later moved to ANTLR4 for the grammar
- A parse-tree-walking interpreter (not a proper AST — my first named mistake)
- A native C backend that translated EZ code to C, then compiled via `clang`
- A "friend module" system for calling into C++: `friend math : cpp as mcpp;`
- An environment system using nix-shell and `.ezconfig` for reproducibility
- Written in C++

My own description at the end of the article:

> "Somewhere between the Minecraft mod and the segfaults, this stopped being a
> language and started being something else — more of a coordination layer."

That sentence is the hinge point between the two projects.

---

## The Fundamental Pivot

**EZ-Language was trying to be a programming language that happened to support
multiple backends.**

**GluCore is a runtime coordination layer that lets me — and you — write in any language.**

Those sound similar. The difference is enormous in practice:

| | EZ-Language | GluCore |
|---|---|---|
| You write code in | EZ syntax | Your native language (Rust, C++, Java, Python) |
| The system's job | Compile/interpret EZ, call out to C++ | Route calls between native modules |
| Type declarations | In EZ grammar | From the native function signature directly |
| Build system | Custom (ANTLR + clang backend) | Delegate to cargo/cmake/javac |
| Entry point | EZ interpreter or C binary | Python orchestration script |

EZ-Language required learning a new language. GluCore requires tagging one function.

---

## Change 1: From Custom Language to No Language

**EZ-Language:** ANTLR4 grammar (`.g4` files), a lexer, a parser, a CST walker, a
C code generator. Thousands of lines of infrastructure before a single cross-language
call could happen.

**GluCore:** No grammar. No lexer. No parser. No interpreter.

**Why:** I named it directly in the article — "the interpreter directly walked the CST
instead of a proper AST" was my first architectural mistake. But the deeper lesson
was that the grammar infrastructure was consuming all my engineering effort before
the actual problem (cross-language calls) was even addressed. The friend module
syntax `friend math : cpp as mcpp;` was genuinely good UX, but maintaining the
parser that understood it was the wrong investment.

The replacement isn't "build a better parser." It's "don't have a parser for the
language of modules at all." I moved module declarations to `glucore.toml` — a format
every developer already knows — and type declarations disappear entirely (see
Change 3).

---

## Change 2: From C++ Implementation to Rust Core

**EZ-Language:** Written in C++. The lexer, parser, interpreter, C backend, and
friend module bridge were all C++.

**GluCore:** Core router written in Rust. Exports a pure C ABI (`libglucore_core.so`)
that every language can call via its own standard FFI mechanism.

**Why two reasons:**

First, C++ gave me every footgun I warned about in the article.
I mentioned "more segfaults and core dumps than I'd like to admit" — that's an
understatement. Rust's ownership model catches at compile time the exact class of
bugs C++ surfaced at runtime — use-after-free, double-free, dangling pointers. That
matters especially for a system explicitly managing cross-language object lifetimes.

Second, Rust's proc-macro system (`#[glucore::export]`) is what makes Change 3
possible. There's no equivalent in C++ that can parse a function signature at
compile time and generate type-safe wrapper code without a separate code-generation
tool. Rust macros let me eliminate the entire ANTLR pipeline for the type-declaration
problem.

The C ABI surface (`extern "C"`, `#[no_mangle]`) means my Rust internals are
invisible to every other language. Python loads it with `ctypes`. C++ links against
it with `target_link_libraries`. Java talks to it over a socket. None of them know
or care it's Rust underneath.

---

## Change 3: From Manual Type Declarations to Zero Duplication

This is the most important single change I made, and the one that directly addressed
my biggest stated failure in EZ-Language.

**EZ-Language:** The friend module system required declaring what a C++ module
exposed inside EZ syntax:

```
friend math : cpp as mcpp;
print(mcpp.add(3, 2));
```

The EZ side had to know about `add`. The C++ side had to implement `add`. Two
places, two declarations, guaranteed drift over time. I acknowledged this
explicitly in the article: "type marshalling for friend calls becoming ugly very quickly."

**GluCore:** One declaration, in the native language, at the function definition. That's it:

```rust
#[glucore::export]
pub fn add(a: f64, b: f64) -> f64 { a + b }
```

The proc-macro reads my Rust function signature at compile time using `syn`, maps
each parameter and return type to a `GluTypeTag`, generates the C-ABI wrapper
function, and emits a static `GluSignature` — all without me writing
anything twice.

C++ gets the same guarantee via templates:

```cpp
double add(double a, double b) { return a + b; }
GLUCORE_EXPORT(add)
```

Java via reflection:

```java
@GluExport
public static double add(double a, double b) { return a + b; }
```

**Why this matters beyond elegance:** An unsupported type in EZ-Language was a
runtime crash or silent wrong behavior that I'd only discover while running the
program. In GluCore, an unsupported type is a compile-time error (`compile_error!`
in Rust, `static_assert` in C++, `IllegalStateException` at registration time in
Java). The error happens before the program runs, names the offending type, and
suggests the correct alternative.

---

## Change 4: From `.gluc` DSL to `glucore.toml` (Topology Only)

**EZ-Language evolution (planned):** An earlier GluCore design iteration
I documented in the Obsidian vault involved a `.gluc` file with a full grammar:

```gluc
module physics {
    language : rust
    build    : "cargo build --release"
    artifact : "./target/release/libphysics.so"
    exports {
        fn calculate_force(mass: GluFloat, accel: GluFloat) -> GluFloat
    }
}
```

I abandoned this before implementation.

**GluCore v2:** `glucore.toml` with only two things — module names and link
permissions:

```toml
[modules]
physics = "physics"

[links]
python = ["physics"]
```

No type signatures. No build commands. No `exports` block. Types come from source
code. Build commands live in the language's own build system. The manifest's only
job is declaring what modules exist and who is allowed to call whom.

**Why:** The `.gluc` exports block was EZ-Language's type-duplication problem
wearing different clothes. Moving type information into the manifest only changes
where the duplication happens — it doesn't eliminate it. I realised the correct
solution is to not put type information in the manifest at all, which is what
`glucore.toml` does.

The `[links]` table is something EZ-Language never had: a permission graph. Friend
modules in EZ had no concept of authorization — any module could call any other.
In GluCore I enforce caller identity at the router level, and the router checks
caller → callee against the declared links before dispatching any call, even before
any IPC bytes are sent.

---

## Change 5: From Parse-Tree Walking to Compile-Time Code Generation

**EZ-Language:** I walked the ANTLR parse tree (CST) directly at
runtime. I named this as my first major architectural mistake in the article:
"there's a big difference between 'this parses correctly' and 'this is
architecturally sane.'"

**GluCore:** There is no parse tree. There is no runtime reflection. The `#[export]`
macro runs at `cargo build` time and emits a `static GluExport` entry collected by
`inventory::submit!`. By the time the `.so` is loaded, every exported function's
name, parameter types, and return type are baked into read-only static memory — I
computed all of that at compile time and the runtime just reads a hash map.

**Why:** Walking a CST at runtime couples execution speed to parsing speed and
couples the type system to the grammar. It also makes the "architecturally insane"
failure mode I hit in EZ-Language — adding grammar features before the runtime knew
what to do with them — structurally impossible. The proc-macro either generates
correct code or the build fails. I can't reach a state of "parses correctly but
behaves wrong" for exported functions anymore.

---

## Change 6: From Single-Language Execution to Three-Language Mesh

**EZ-Language:** I supported calling C++ from EZ via friend modules. Python support
existed but was "rougher around the edges" — my own words from the article. The
execution model was fundamentally one language (EZ) calling out to one other (C++).

**GluCore v2:** A verified three-language call chain in a single runtime:

```
Python → physics (Rust) → cpp_engine (C++) → java_renderer (Java)
```

Each arrow is a real routing decision by the Rust core, a real link-permission
check, and a different physical mechanism:

- Python → Rust: `ctypes` calling `glucore_call()` in `libglucore_core.so`
- Rust → C++: `glucore_call()` calling a function pointer registered at load time via `dlopen`
- C++ → Java: `glucore_call()` serializing a FCTP packet over a Unix domain socket

The three-language mesh isn't a demo claim — I have a DoD verification script
(`scripts/task9_dod.py`) that runs all four calls in a single process instance and
reports literal return values.

**Why three module kinds exist where EZ had one:**

My EZ-Language friend module system only modeled `ARTIFACT` (compiled, loaded) modules.
I added:

- `SOURCE` — interpreted languages (Python, Lua) that run directly; no compilation step
- `PROCESS` — languages that can't be `dlopen`'d (JVM, .NET, Go); spawned as a child
  process and reached over a Unix socket

The `PROCESS` kind is what Java needs. A JVM can't be loaded as a shared library into
another process. EZ-Language had no answer for this — it only worked with C++ because
C++ produces `.so` files. I had to design my way around that.

---

## Change 7: From No Wire Protocol to Explicit Binary Protocol (IPC)

**EZ-Language:** All cross-language calls were in-process. The friend module was
loaded into the same process as EZ. No serialization, no network.

**GluCore (PROCESS modules):** A binary wire protocol over Unix domain sockets:

```
Call message (Rust → Java):
  [1 byte  msg_type = 0x00]
  [4 bytes payload_len, u32 LE]
  [4 bytes module name len][bytes]
  [4 bytes fn name len][bytes]
  [4 bytes caller id len][bytes]
  [1 byte  arg count]
  repeated: [1 byte GluTypeTag][payload by tag]
```

This exists entirely because Java — and any other JVM or managed-runtime language —
can't participate in the same process as a `dlopen`'d module. I wrote the protocol
spec down once (`glucore_core/src/ipc_transport.rs`) to prevent Rust
and Java from drifting into two incompatible interpretations — a failure mode that
actually happened to me once during development (documented as Footgun #6 and #7 in
`KNOWN_FOOTGUNS.md`).

---

## Change 8: From "Works on My Machine" as an Aspiration to a Documented Gap

**EZ-Language:** The `.ezconfig` and nix-shell environment system was one of my
more forward-looking ideas in EZ — I explicitly credited NixOS in the article with
the insight that environments should be part of the program contract, not a README
that nobody reads.

**GluCore v2:** Nix is deferred. The `build.sh` checks for toolchain presence
(`cargo`, `cmake`, `javac`) and fails with a clear message if something is missing,
but it doesn't create isolated environments. Linux and macOS are tested; Windows is
explicitly unsupported with a documented reason (IPC uses Unix domain sockets, and
the named-pipe equivalent hasn't been implemented).

**Why the deferral:** EZ-Language taught me that trying to solve environment
reproducibility, cross-language calling, and language implementation simultaneously
means none of them get solved properly. I chose to prove the calling mechanism first
in GluCore. The environment story is Phase 3.

**What stayed from my original insight:** The instinct that "clone project → run
project" is a real developer need — I still believe that. The build script
verifies its own dependencies and runs a smoke test at the end rather than
documenting setup steps in a README that will inevitably drift from reality.

---

## What Stayed the Same

Some things survived unchanged across both projects:

**The original problem statement.** A late-night Minecraft mod question — "why can't
I use C++ for this heavy computation without rewriting everything?" — is still the
exact use case I'm building for.

**The friend module concept's spirit.** My `friend math : cpp as mcpp` syntax
was declaring that a C++ module existed and giving it a local name. GluCore's
`[modules]` table in `glucore.toml` does the same thing with less syntax. The idea
that I should be able to name a module and use it without manual linking commands
survived the full rewrite, even though the syntax changed completely.

**The environment-as-contract insight.** My `.ezconfig` vision — that which compiler,
which version, which dependencies are all part of the project and shouldn't live in
a README — is still the right idea. I haven't implemented it in GluCore yet, but
it's in the roadmap.

**The honest acknowledgment that FFI is hard.** I wrote in the article: "every
abstraction eventually reaches the FFI wall." My `KNOWN_FOOTGUNS.md` with eight
documented failures is the operational version of that same observation. The wall
didn't go away. The difference is that now I document every time I hit it and what
I learned, rather than listing it as a known future problem.

---

## What Was Learned and Directly Applied

I listed EZ-Language's problems explicitly in the article. Every one of them drove a
specific GluCore design decision:

| EZ-Language problem | GluCore design response |
|---|---|
| CST walking instead of proper AST | No interpreter at all — proc-macro generates code at `cargo build` time |
| Type marshalling becoming ugly quickly | `GluTypes` + `#[export]` macro — types come from the function signature, never declared separately |
| Segfaults and core dumps | Rust core with `catch_unwind` at every FFI boundary — a panic returns `GluStatus::Runtime`, never crashes the host process |
| Parser/runtime/features growing faster than architecture | Phase-gated roadmap — prove `Python ↔ Rust` before adding C++, prove C++ before adding Java |
| Architecture growing slower than features | Minimum first: one type, one transport, one language pair, then expand |
| "Works on my machine" | Explicit platform support table in README; `KNOWN_FOOTGUNS.md` with platform-specific failure modes |
| Adding grammar features before implementing them properly | No grammar. If it doesn't build, it doesn't exist. |

---

## The Numbers

EZ-Language has no performance measurements — I was still in the "does it work"
phase. GluCore has measured overhead:

| Path | Per-call latency | Calls/sec |
|---|---|---|
| Raw ctypes (no GluCore) | 0.633 µs | 1,578,659 |
| GluCore v2 (optimized) | 3.002 µs | 333,095 |
| GluCore v1 (unoptimized) | 51.6 µs | 19,397 |

The 4.7× overhead relative to raw ctypes is the cost of GluCore's guarantees:
Buffer copy-in/copy-out, pre-call type validation, and link permission check. The
37× overhead in my first version was fixed by eliminating Python-side closure
reconstruction on every call, pre-allocating ctypes arrays, and removing allocations
in the Rust dispatch path.

These numbers don't exist for EZ-Language because I never reached the
point where performance was the question. With GluCore, I did.

---

## Summary

EZ-Language was my experiment in building a language that could coordinate other
languages. I learned, at real cost in segfaults and architectural debt, that the
language itself was the wrong level of abstraction. The cross-language coordination
problem is real; writing an interpreter to solve it is not.

GluCore takes the same origin problem, the same core vision, and the specific
lessons I paid for in sweat, then rebuilds from a different starting
point: the native function signature is the contract, the runtime only routes,
and every layer does exactly one thing.

Whether that's better is measurable now, where it wasn't before. The mesh
works — three languages, one call chain, verified with literal output in a
single run. That's the further-than-expected distance from a late-night coffee
session I was already celebrating in the EZ-Language article. GluCore just went
further.
