{ self, description }:
{
  config,
  lib,
  pkgs,
  ...
}:
let
  pname = "cantus";
  cfg = config.programs.cantus;
  settingsFormat = pkgs.formats.toml { };
  settingsOptions = import ./generated-options.nix { inherit lib; };
in
{
  options.programs.cantus = {
    enable = lib.mkEnableOption description;

    package = lib.mkOption {
      type = lib.types.package;
      default = self.packages.${pkgs.stdenv.hostPlatform.system}.cantus;
      defaultText = lib.literalExpression "inputs.${pname}.packages.${pkgs.stdenv.hostPlatform.system}.${pname}";
      description = "Cantus package to install.";
    };

    autoStart = lib.mkOption {
      type = lib.types.bool;
      default = true;
      description = "Whether to start the Cantus widget automatically.";
    };

    settings = lib.mkOption {
      type = lib.types.nullOr (
        lib.types.submodule {
          options = settingsOptions;
        }
      );
      default = null;
      description = "Settings written as TOML to `~/.config/cantus/cantus.toml`.";
      example = lib.mapAttrs (_: option: option.default) settingsOptions;
    };
  };

  config = lib.mkIf cfg.enable {
    home.packages = [ cfg.package ];

    xdg.configFile = lib.optionalAttrs (cfg.settings != null) {
      "cantus/cantus.toml".source = settingsFormat.generate "cantus.toml" (
        lib.filterAttrs (_: value: value != null) cfg.settings
      );
    };

    systemd.user.services.cantus = lib.mkIf cfg.autoStart {
      Unit = {
        Description = description;
        After = [ config.wayland.systemd.target ];
        X-Restart-Triggers = lib.optional (
          cfg.settings != null
        ) config.xdg.configFile."cantus/cantus.toml".source;
      };

      Service = {
        Type = "simple";
        ExecStart = "${cfg.package}/bin/${pname}";
        Restart = "on-failure";
      };

      Install.WantedBy = [ config.wayland.systemd.target ];
    };
  };
}
