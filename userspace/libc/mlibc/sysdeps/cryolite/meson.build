sysdep_supported_options = {
  'posix': false,
  'linux': false,
  'glibc': false,
  'bsd': false,
}

port_includes = include_directories('include')
rtld_include_dirs = [port_includes] + rtld_include_dirs
libc_include_dirs = [port_includes] + libc_include_dirs
rtld_sources += files('sysdeps.cpp')
libc_sources += files('entry.cpp', 'sysdeps.cpp', 'files.cpp', 'rust.cpp', 'threads.cpp', 'threads.S')

if not no_headers
  install_subdir('include', install_dir: get_option('includedir'), strip_directory: true)
endif

if not headers_only
  custom_target('crt1',
    build_by_default: true,
    command: c_compiler.cmd_array() + freestanding_hdrs_args
      + ['-c', '-mno-red-zone', '-o', '@OUTPUT@', '@INPUT@'],
    input: 'crt1.S',
    output: 'crt1.o',
    install: true,
    install_dir: get_option('libdir'))
endif
