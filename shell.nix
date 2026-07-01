# shell.nix — Nix development environment for GluCore v2.
#
# Enter with: nix-shell shell.nix
# Then run:   ./build.sh

{ pkgs ? import <nixpkgs> {} }:

pkgs.mkShell {
  buildInputs = with pkgs; [
    # Rust toolchain
    rustc
    cargo
    rustfmt
    clippy

    # C/C++ toolchain
    gcc
    cmake

    # Java
    jdk17_headless

    # Python
    python311

    # Useful tools
    pkg-config
    ldd
  ];

  # Set up the environment so cargo and cmake can find everything.
  shellHook = ''
    echo "GluCore v2 development environment"
    echo "Run './build.sh' to build everything."
    echo ""
    echo "Rust:    $(rustc --version)"
    echo "CMake:   $(cmake --version | head -1)"
    echo "Java:    $(javac --version)"
    echo "Python:  $(python3 --version)"
  '';
}
