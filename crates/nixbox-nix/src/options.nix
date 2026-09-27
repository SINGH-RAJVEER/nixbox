# Lists the module options for one installed package.
#
# Evaluated with `nix eval --impure --json --file`; the query arrives as JSON in
# NIXBOX_OPTIONS_QUERY so no value is ever spliced into Nix source:
#
#   root       configuration directory holding flake.nix
#   source     "nixos", "home", or "home-in-nixos"
#   host       nixosConfigurations attribute
#   user       homeConfigurations attribute, or home-manager.users attribute
#   package    package attribute, such as "git"
#   candidates namespaces to try first, such as [ [ "programs" "git" ] ]
let
	query = builtins.fromJSON (builtins.getEnv "NIXBOX_OPTIONS_QUERY");
	flake = builtins.getFlake (toString query.root);
	nixos = flake.nixosConfigurations.${query.host};

	# Where the options live, and how many leading `loc` segments come before
	# the option paths a user writes in their own module.
	target =
		if query.source == "nixos" then {
			options = nixos.options;
			lib = nixos.pkgs.lib;
			prefix = 0;
		} else if query.source == "home" then
			let home = flake.homeConfigurations.${query.user};
			in {
				options = home.options;
				lib = home.pkgs.lib;
				prefix = 0;
			}
		else {
			options = nixos.options.home-manager.users.valueMeta.attrs.${query.user}.configuration.options;
			lib = nixos.pkgs.lib;
			prefix = 3;
		};
	inherit (target) lib options;

	isNamespace = path:
		let found = builtins.tryEval (
			lib.hasAttrByPath path options
			&& builtins.isAttrs (lib.getAttrFromPath path options)
			&& !lib.isOption (lib.getAttrFromPath path options)
		);
		in found.success && found.value;

	# Namespaces whose `package` option defaults to this package, for names
	# that differ from the attribute, such as `programs.neovim` for
	# `neovim-unwrapped`.
	byPackage =
		let
			wanted = "pkgs.${query.package}";
			scan = group:
				let children = builtins.tryEval (builtins.attrNames (options.${group} or { }));
				in builtins.filter (name:
					let
						option = options.${group}.${name};
						text = builtins.tryEval (
							if builtins.isAttrs option && option ? package && lib.isOption option.package
							then (option.package.defaultText.text or option.package.defaultText or null)
							else null
						);
					in text.success && text.value == wanted
				) (if children.success then children.value else [ ]);
		in
			lib.concatMap (group: map (name: [ group name ]) (scan group)) [ "programs" "services" ];

	direct = builtins.filter isNamespace query.candidates;
	namespaces = if direct != [ ] then direct else byPackage;

	# Values nixbox can show: derivations and functions become markers, and
	# anything nested deeper than `depth` is cut off.
	safe = depth: value:
		let kind = builtins.typeOf value;
		in
			if kind == "lambda" then { _nixbox = "function"; }
			else if lib.isDerivation value then {
				_nixbox = "derivation";
				name = let name = builtins.tryEval (value.name or ""); in if name.success then name.value else "";
			}
			else if kind == "set" then
				if depth == 0 then { _nixbox = "truncated"; }
				else builtins.mapAttrs (_: safe (depth - 1)) value
			else if kind == "list" then
				if depth == 0 then { _nixbox = "truncated"; }
				else map (safe (depth - 1)) value
			else if kind == "path" then toString value
			else value;

	attempt = value:
		let result = builtins.tryEval (builtins.deepSeq value value);
		in if result.success then { ok = result.value; } else { failed = true; };

	describeType = depth: type: {
		name = type.name or "unspecified";
		description = type.description or type.name or "unspecified";
		elem =
			if depth > 0 && type ? nestedTypes && type.nestedTypes ? elemType
			then describeType (depth - 1) type.nestedTypes.elemType
			else null;
		values =
			if (type.name or "") == "enum" then
				let payload = type.functor.payload or [ ];
				in if builtins.isList payload then payload else payload.values or [ ]
			else null;
	};

	literal = value:
		if value == null then null
		else if builtins.isAttrs value && value ? text then value.text
		else if builtins.isString value then value
		else null;

	# The option a doc entry describes, or null for sub-options of a
	# submodule option, which are not reachable as plain attributes.
	# `getAttrFromPath` would abort on those, and aborts cannot be caught.
	optionAt = doc: lib.attrByPath (lib.drop target.prefix doc.loc) null options;

	describe = doc:
		let
			path = lib.drop target.prefix doc.loc;
			option = optionAt doc;
		in {
			inherit path;
			description = attempt (doc.description or null);
			readOnly = doc.readOnly or false;
			type = attempt (describeType 2 option.type);
			default = attempt (literal (doc.default or null));
			example = attempt (literal (doc.example or null));
			value = attempt (safe 3 option.value);
			files = attempt (map (definition: toString definition.file) (option.definitionsWithLocations or [ ]));
			# 100 for plain definitions, 1000 for mkDefault, 1500 for the option's own default.
			priority = attempt (option.highestPrio or 1500);
		};

	visible = doc:
		!(doc.internal or false)
		&& (doc.visible or true) != false
		&& !(builtins.elem "*" doc.loc)
		&& !(builtins.elem "<name>" doc.loc)
		&& lib.isOption (optionAt doc);

	docs = lib.concatMap
		(path: builtins.filter visible (lib.optionAttrSetToDocList (lib.getAttrFromPath path options)))
		namespaces;
in {
	root = toString flake.outPath;
	inherit namespaces;
	options = map describe docs;
}
