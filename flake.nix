{
  inputs.nelhage.url = "github:nelhage/nix-config";
  inputs.nixpkgs.follows = "nelhage/nixpkgs";

  outputs =
    { nixpkgs, ... }:
    let
      forAllSystems = nixpkgs.lib.genAttrs [
        "aarch64-linux"
        "x86_64-linux"
        "aarch64-darwin"
      ];
    in
    {
      devShells = forAllSystems (
        system:
        let
          pkgs = nixpkgs.legacyPackages.${system};
        in
        {
          default =
            let
              python = pkgs.python314;
            in
            pkgs.mkShell {
              packages = [
                python
                pkgs.uv
                pkgs.cargo
                pkgs.rustc
                pkgs.clippy
                pkgs.rustfmt
                pkgs.rust-analyzer
              ];
              UV_PYTHON = "${python}/bin/python";
            };
        }
      );
    };
}
