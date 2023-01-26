# msvc-dev-cmd

A Rust program to run any command under the [Visual Studio Developer Command
Prompt](https://learn.microsoft.com/en-us/visualstudio/ide/reference/command-prompt-powershell)
of your choice.

This is a port of the GitHub Action by ilammy: https://github.com/ilammy/msvc-dev-cmd,
but designed for desktop use.

## Using

```
Usage: msvc-dev-cmd [OPTIONS] <PROGRAM> [ARGS]...

Arguments:
  <PROGRAM>  Name or path to the program I'll background to
  [ARGS]...  Arguments to the program

Options:
      --arch <ARCH>            Target architecture [default: x64]
      --sdk <SDK>              Windows SDK number to build for
      --spectre                Enable Spectre mitigations
      --toolset <TOOLSET>      VC++ compiler toolset version
      --uwp                    Build for Universal Windows Platform
      --vsversion <VSVERSION>  The Visual Studio version to use. This can be the version number (e.g. 16.0 for 2019) or the year (e.g. "2019")
  -h, --help                   Print help
  -V, --version                Print version
```

## Building

Just `cargo build`. No special Rust features are required.

## License

Mozilla Public License 2.0
