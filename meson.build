project('qdl', 'c',
  default_options : ['warning_level=2', 'optimization=2'],
  license: 'BSD-3-Clause',
  meson_version: '>=0.56.0'
)

# Use 64-bit file offsets everywhere. This must be consistent across every
# translation unit: off_t appears in shared types (e.g. struct firehose_op)
# and in the qdl_file_seek() ABI, so a per-file definition would give
# mismatched struct layouts on 32-bit builds.
add_project_arguments('-D_FILE_OFFSET_BITS=64', language : 'c')

# Determine version
env_v = get_option('VERSION')
if env_v != ''
  ver = env_v
else
  git = find_program('git', required: false)
  git_v = 'unknown-version'
  if git.found()
    git_check_ver = run_command(git, 'describe', '--dirty', '--always', '--tags', check: false).stdout().strip()
    git_v = git_check_ver != '' ? git_check_ver : git_v
  endif
  ver = git_v
endif

# Dependencies
libusb_dep = dependency('libusb-1.0')
libxml_dep = dependency('libxml-2.0')
libzip_dep = dependency('libzip', required: get_option('zip-container'))
help2man = find_program('help2man', required: false)
cmocka_dep = dependency('cmocka', required: get_option('tests'))

# Rust is a hard build requirement: the rust/ workspace is compiled into
# libqdl_rs.a by cargo. Dependency lookup happens here so
# a missing toolchain fails with a pointer to the install instructions.
cargo = find_program('cargo', required: false)
if not cargo.found()
  error('cargo is required to build qdl; install Rust from https://rustup.rs')
endif

# Public API headers live in include/, implementation headers in src/.
# '.' adds the top-level build dir, where configure_file generates
# version.h, so targets in subdirs (e.g. tests/) resolve it too.
inc = include_directories('include', 'src', '.')

# Windows-specific link flags
ws2_dep = []
setupapi_dep = []
if host_machine.system() == 'windows'
  ws2_dep = meson.get_compiler('c').find_library('ws2_32', required : false)
  # SetupAPI / cfgmgr32 are needed by the QUD backend to enumerate Qualcomm
  # COM ports exposed by the official QDLoader 9008 driver.
  setupapi_dep = [
    meson.get_compiler('c').find_library('setupapi', required : false),
    meson.get_compiler('c').find_library('cfgmgr32', required : false),
  ]
endif

# --- Rust static library ---
# cargo builds the rust/ workspace in release mode. The C prototypes for
# the #[export] functions are hand-written in their C headers, so the C
# sources do not wait for cargo; only the link does. build_always_stale
# lets cargo decide what to rebuild.
rust_manifest = join_paths(meson.project_source_root(), 'rust', 'qdl-rs', 'Cargo.toml')
rust_script = join_paths(meson.project_source_root(), 'scripts', 'build_rust.sh')
rust_lib = custom_target('qdl_rs',
  command: ['sh', rust_script, cargo.full_path(), rust_manifest,
            '@OUTDIR@/rust-target', '@OUTPUT@'],
  output: 'libqdl_rs.a',
  build_always_stale: true,
)

# std needs the unwinder from libgcc_s when linked statically. glibc 2.34 and
# later provide pthread, dl, rt, util and libm inside libc, so only
# libgcc_s is listed. Each entry is optional, so the list also works on
# musl and macOS.
rust_sys_names = ['gcc_s']
if host_machine.system() == 'windows'
  rust_sys_names = ['ws2_32', 'userenv', 'bcrypt', 'ntdll', 'advapi32']
endif
rust_sys_libs = []
foreach name : rust_sys_names
  lib = meson.get_compiler('c').find_library(name, required: false)
  if lib.found()
    rust_sys_libs += lib
  endif
endforeach

# The static library is linked by path, since meson cannot link a custom
# target directly. Executables that link it list rust_lib in link_depends
# so cargo runs first.
rust_dep = declare_dependency(
  link_args: [rust_lib.full_path()],
  dependencies: rust_sys_libs,
)

common_dep = [libusb_dep, libxml_dep, ws2_dep, setupapi_dep, rust_dep]
compile_view_deps = [libusb_dep, libxml_dep]

# Zip container support is optional; the src subdirectory swaps the
# libzip-backed sources for no-zip stubs when it is disabled.
if libzip_dep.found()
  common_dep += libzip_dep
  compile_view_deps += libzip_dep
endif

# Compile-only view of common_dep (includes + cflags, no link args).
# Used on executables so their own sources see the headers, while link
# args propagate exactly once through the static library below, avoiding
# duplicate "-l..." entries on the link line.
common_compile_dep = []
foreach d : compile_view_deps
  common_compile_dep += d.partial_dependency(compile_args: true, includes: true)
endforeach

# --- version header generation ---
conf = configuration_data()
conf.set('version', ver)

version_h = configure_file(
  input: 'include/version.h.in',
  output: 'version.h',
  configuration: conf
)

# Source lists (common_sources, qdl_sources and the per-test source
# vars) are declared next to the sources.
subdir('src')

shared_lib = static_library('qdl_common', common_sources,
  dependencies: common_dep,
  include_directories: inc,
)

# --- qdl executable ---
qdl_exe = executable('qdl',
  sources : qdl_sources + [version_h],
  link_with: shared_lib,
  link_depends: rust_lib,
  dependencies : common_compile_dep,
  include_directories : inc,
  install : true
)

# --- udev rules ---
# Prefer the rules directory advertised by udev.pc so a /usr/local prefix
# still installs where udev looks; packagers can override it explicitly.
if host_machine.system() == 'linux' and not get_option('udev').disabled()
  udevrulesdir = get_option('udevrulesdir')
  if udevrulesdir == ''
    udev_dep = dependency('udev', required : false)
    if udev_dep.found()
      udevrulesdir = udev_dep.get_variable(pkgconfig : 'udevdir') / 'rules.d'
    else
      udevrulesdir = get_option('prefix') / 'lib' / 'udev' / 'rules.d'
    endif
  endif

  # Device access is granted to the active local session via uaccess; an
  # additional group (e.g. plugdev on Debian) is opt-in as group names
  # differ between distributions.
  udev_conf = configuration_data()
  udev_group = get_option('udev_group')
  if udev_group != ''
    udev_conf.set('UDEV_GROUP_ATTRS', 'MODE="0660", GROUP="' + udev_group + '", ')
  else
    udev_conf.set('UDEV_GROUP_ATTRS', '')
  endif

  configure_file(
    input : 'udev/70-qdl.rules.in',
    output : '70-qdl.rules',
    configuration : udev_conf,
    install_dir : udevrulesdir
  )
endif

# --- optional nbdkit plugin ---
# nbdkit dlopens plugins, so there is no library to link against; the plugin
# just needs <nbdkit-plugin.h> and compiles all non-main sources into a shared
# module named exactly "nbdkit-qdl-plugin.so".
nbdkit_dep = dependency('nbdkit', required: get_option('nbdkit'))
nbdkit_plugin = []
if nbdkit_dep.found()
  nbdkit_plugin = shared_module('nbdkit-qdl-plugin',
    sources : nbdkit_plugin_src + lib_sources + common_sources + [version_h],
    link_depends : rust_lib,
    dependencies : common_dep + [nbdkit_dep],
    include_directories : inc,
    name_prefix : '',
    install : true,
    install_dir : get_option('libdir') / 'nbdkit' / 'plugins',
  )
endif

# --- tests ---
subdir('tests')

# --- checkpatch targets  ---
check_script = join_paths(meson.project_source_root(), 'scripts', 'checkpatch_wrapper.sh')
foreach checkpatchparam : ['download-checkpatch', 'check', 'check-cached', 'check-range']
  run_target(checkpatchparam,
    command: ['bash', '-lc', check_script + ' ' + checkpatchparam]
  )
endforeach

# --- markdown lint target ---
markdown_script = join_paths(meson.project_source_root(), 'scripts', 'markdownlint_wrapper.sh')
run_target('markdown-lint',
  command: ['bash', '-lc', markdown_script + ' check']
)

# --- man generation ---
if help2man.found()
  # Test if we can run the built executable (might fail in cross-compilation)
  can_run_exe = not meson.is_cross_build()

  if can_run_exe
    manpage_targets = []
    foreach prog : [
      ['qdl', qdl_exe, 'Qualcomm Download'],
    ]
      manpage_targets += custom_target(prog[0] + '.1',
        input: prog[1],
        output: prog[0] + '.1',
        command: [help2man, '-N', '-n', prog[2], prog[1].full_path()],
        capture: true,
        install: true,
        install_dir: get_option('mandir') / 'man1')
    endforeach

    # Replicate the "manpages" phony target. alias_target requires at
    # least one dependency, so it is only declared when we have actually
    # produced manpage targets above.
    alias_target('manpages', manpage_targets)
  endif
endif
