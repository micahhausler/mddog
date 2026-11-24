{
  description = "Render Markdown in the terminal as a browser shows them, styled like GitHub by default";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
    # nixpkgs-unstable (26.11) dropped x86_64-darwin; 26.05 is the last branch
    # supporting Intel Macs so we pin to that.
    nixpkgs-26_05-darwin.url = "github:NixOS/nixpkgs/nixpkgs-26.05-darwin";
  };

  outputs =
    {
      self,
      nixpkgs,
      nixpkgs-26_05-darwin,
    }:
    let
      inherit (nixpkgs) lib;
      systems = [
        "aarch64-darwin"
        "x86_64-darwin"
        "aarch64-linux"
        "x86_64-linux"
      ];

      mkPkgs =
        system:
        import (if system == "x86_64-darwin" then nixpkgs-26_05-darwin else nixpkgs) {
          inherit system;
          config = {
            allowUnfree = true;
            allowBroken = system == "x86_64-darwin";
          };
        };

      forAllSystems = f: lib.genAttrs systems (system: f (mkPkgs system) system);

      cargoToml = fromTOML (builtins.readFile ./Cargo.toml);
      fontsConf =
        pkgs:
        pkgs.makeFontsConf {
          fontDirectories = with pkgs; [
            dejavu_fonts
            noto-fonts
            noto-fonts-color-emoji
            stix-two
          ];
        };
    in
    {
      packages = forAllSystems (
        pkgs: system: {
          mddog = pkgs.rustPlatform.buildRustPackage {
            pname = cargoToml.package.name;
            version = cargoToml.package.version;

            src = pkgs.lib.cleanSource ./.;

            cargoLock = {
              lockFile = ./Cargo.lock;
            };

            FONTCONFIG_FILE = pkgs.lib.optionalString pkgs.stdenv.hostPlatform.isLinux (fontsConf pkgs);

            nativeBuildInputs = pkgs.lib.optionals pkgs.stdenv.hostPlatform.isLinux [ pkgs.makeWrapper ];

            postInstall = pkgs.lib.optionalString pkgs.stdenv.hostPlatform.isLinux ''
              wrapProgram $out/bin/mdd --set-default FONTCONFIG_FILE ${fontsConf pkgs}
            '';

            checkFlags = [
              # Requires macOS /usr/bin/defaults (unavailable in the Nix sandbox).
              "--skip=help_follows_an_appearance_change"
              # Invokes macOS BSD `base64 -D`, whereas Nix's stdenv provides GNU `base64`.
              "--skip=cmd_c_copies_the_selection"
              "--skip=copy_on_select_copies_a_double_clicked_word"
              "--skip=copy_on_select_copies_a_drag"
            ];

            meta = {
              description = cargoToml.package.description;
              homepage = "https://github.com/qu8n/mddog";
              license = pkgs.lib.licenses.mit;
              platforms = systems;
              mainProgram = "mdd";
            };
          };

          default = self.packages.${system}.mddog;
        }
      );

      devShells = forAllSystems (
        pkgs: system: {
          default = pkgs.mkShell {
            inputsFrom = [ self.packages.${system}.mddog ];
            packages = with pkgs; [
              cargo
              clippy
              rustc
              rustfmt
            ];
          };
        }
      );
    };
}
