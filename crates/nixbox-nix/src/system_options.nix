# Read the selected host's schema separately from its values. The request is
# JSON in the environment, so host names and paths are never source fragments.
let
	query = builtins.fromJSON (builtins.getEnv "NIXBOX_SYSTEM_OPTIONS_QUERY");
	flake = builtins.getFlake (toString query.root);
	host = flake.nixosConfigurations.${query.host};
	lib = host.pkgs.lib;

	attempt = value:
		let result = builtins.tryEval (builtins.deepSeq value value);
		in if result.success then result.value else null;

	# Namespace depth does not consume this budget. Only expanding a submodule
	# does, so recursive container schemas cannot grow the index indefinitely.
	walk = depth: diagnostic: nested: inheritedConfigured: prefix: tree:
		lib.concatMap (name:
			let
				node = tree.${name};
				path = if lib.isOption node then node.loc or (prefix ++ [ name ]) else prefix ++ [ name ];
				visibility = node.visible or true;
				isDiagnostic = diagnostic || (node.internal or false);
				# Inspect definition provenance, never the definition values. A
				# parent contribution also retains its nested schema fields.
				configured = if inheritedConfigured != false then inheritedConfigured
					else if nested then false else attempt (
						(node.highestPrio or 1500) < 1500 && builtins.any (definition:
							let file = toString definition.file;
							in !(lib.hasPrefix (toString (host.pkgs.path or "/nixbox-no-nixpkgs-path") + "/") file)
						) (node.definitionsWithLocations or [ ])
					);
				children = if depth == 0 then null else
					let
						sub = node.type.getSubOptions path;
						probe = builtins.tryEval (builtins.attrNames sub);
					in if probe.success then sub else null;
				hasSubmodules = attempt ((node.type.getSubModules or null) != null);
				# Container and specialisation configs embed a complete system. Keep
				# their declaration without copying all of NixOS into each branch.
				embeddedSystem = children != null && children ? boot && children ? system && children ? networking;
				row = {
					inherit path nested configured;
					description = attempt (node.description or null);
					typeDescription = let text = attempt (node.type.description or "unknown"); in if text == null then "unknown" else text;
					declarations = let files = attempt (map toString (node.declarations or [ ])); in if files == null then [ ] else files;
					diagnostic = isDiagnostic || visibility == false || visibility == "transparent";
					readOnly = node.readOnly or false;
					template = builtins.elem "<name>" path || builtins.elem "*" path;
					childrenUnavailable = depth > 0 && children == null;
					childrenTruncated = (depth == 0 && hasSubmodules == true) || embeddedSystem;
				};
			in
				if lib.isOption node then
					[ { metadata = row; option = node; } ]
					++ lib.optionals (children != null && children != { } && !embeddedSystem) (
						walk (depth - 1) (isDiagnostic || visibility == false || visibility == "shallow") true configured path children
					)
				else if builtins.isAttrs node then walk depth diagnostic nested inheritedConfigured path node
				else [ ]
		) (builtins.attrNames tree);

	entries = walk 4 false false false [ ] host.options;
	sensitiveSegment = segment:
		builtins.match ".*(password|secret|token|privatekey|private-key).*" (lib.toLower segment) != null;

	# Bound both depth and collection size. An option containing packages or
	# functions must not serialize the package set or force its full closure.
	safe = depth: value:
		let kind = builtins.typeOf value;
		in
			if kind == "lambda" then { _nixbox = "function"; }
			else if lib.isDerivation value then {
				_nixbox = "derivation";
				name = attempt (value.name or "");
			}
			else if kind == "set" || kind == "list" then
				if depth == 0 then { _nixbox = "truncated"; }
				else if kind == "list" then {
					_nixbox = "list";
					items = map (safe (depth - 1)) (lib.take 20 value);
					truncated = builtins.length value > 20;
				}
				else let names = builtins.attrNames value; in {
					_nixbox = "attrs";
					items = builtins.listToAttrs (map (name: {
						inherit name;
						value = if sensitiveSegment name then { _nixbox = "sensitive"; }
							else safe (depth - 1) value.${name};
					}) (lib.take 20 names));
					truncated = builtins.length names > 20;
				}
			else if kind == "path" then toString value
			else if kind == "string" && builtins.stringLength value > 4096 then {
				_nixbox = "string";
				preview = builtins.substring 0 4096 value;
				truncated = true;
			}
			else value;

	# Paths as JSON make unambiguous keys, even for names containing dots.
	index = builtins.listToAttrs (map (entry: {
		name = builtins.toJSON entry.metadata.path;
		value = entry;
	}) entries);
	lookup = path: index.${builtins.toJSON path} or null;

	renderLiteral = value:
		let text = if builtins.isAttrs value && value ? _type && value ? text then value.text
			else lib.generators.toPretty { multiline = true; } (safe 3 value);
		in if builtins.stringLength text > 4096 then builtins.substring 0 4096 text + "\n... truncated" else text;

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

	inspect = path: selected:
		let
			sensitive = builtins.any sensitiveSegment path;
			option = selected.option;
			valueResult = builtins.tryEval (builtins.deepSeq (safe 3 option.value) (safe 3 option.value));
		in {
			inherit path;
			default = if sensitive then null else attempt (
				if option ? defaultText then renderLiteral option.defaultText
				else if option ? default then renderLiteral option.default
				else null
			);
			example = if sensitive then null else attempt (
				if option ? example then renderLiteral option.example else null
			);
			value =
				if sensitive then { status = "sensitive"; }
				else if selected.metadata.nested then { status = "template"; }
				else if valueResult.success then { status = "available"; value = valueResult.value; }
				else { status = "unavailable"; };
			definitions = if selected.metadata.nested || sensitive then null else attempt (
				map (definition: toString definition.file) (option.definitionsWithLocations or [ ])
			);
		};

	# What deciding whether nixbox may write an option needs: its type, and the
	# priority it is defined at. 100 is a plain definition, 1000 mkDefault, and
	# 1500 the option's own default.
	editing = selected: {
		type = attempt (describeType 2 selected.option.type);
		readOnly = selected.metadata.readOnly;
		priority = attempt (selected.option.highestPrio or 1500);
	};

	selected = lookup query.path;
in
	if query.mode == "hosts" then builtins.attrNames (flake.nixosConfigurations or { })
	else if query.mode == "catalog" then { options = map (entry: entry.metadata) entries; }
	else if query.mode == "group" then {
		root = toString flake.outPath;
		options = lib.concatMap (path:
			let found = lookup path;
			in if found == null then [ ] else [ (inspect path found // editing found) ]
		) query.paths;
	}
	else if selected == null then throw "The selected option is no longer declared. Reload the schema."
	else inspect query.path selected
