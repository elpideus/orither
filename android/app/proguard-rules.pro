# Called from Rust by name.
-keep class dev.orither.link.Native { *; }
-keep class dev.orither.link.Native$* { *; }

# Started by name through app_process.
-keep class dev.orither.engine.Main { public static void main(java.lang.String[]); }
