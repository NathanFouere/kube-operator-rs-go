{
  description = "Nix flake";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
  };

  outputs = inputs@{ flake-parts, ... }:
    flake-parts.lib.mkFlake { inherit inputs; } {
      systems = [
        "x86_64-linux"
      ];
      perSystem = { config, self', inputs', pkgs, system, ... }: {
        devShells.default = pkgs.mkShell {
          packages = with pkgs; [
            kubectl
            go
            gopls
            kind
            kubebuilder
            rustc
            cargo
            nil
            rustfmt
            rust-analyzer
            clippy
            kubernetes-helm
          ];
        };
      };
    };
}
