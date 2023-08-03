{
    description = "pulldown cmark with wikilinks";

    inputs = {
        rust-overlay.url = github:oxalica/rust-overlay;
        naersk.url = github:nix-community/naersk;
        utils.url = github:numtide/flake-utils;
    };

    outputs = { self, rust-overlay, nixpkgs, naersk, utils }: 
        with utils.lib;
        eachDefaultSystem (system:
            let overlays = [rust-overlay.overlays.default];
                pkgs = import nixpkgs {inherit system overlays;};
                rust-toolchain = pkgs.rust-bin.nightly.latest.minimal;
                buildPackage = (
                pkgs.callPackage naersk {
                    cargo = rust-toolchain;
                    rustc = rust-toolchain;
                }).buildPackage;
            in
            {
                packages.default = buildPackage {
                    src = ./.;
                };
                devShell = pkgs.mkShell {
                    nativeBuildInputs = [
                        rust-toolchain
                        pkgs.rust-analyzer
                    ];
                };
            }
    );
}
