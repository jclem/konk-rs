# Konk

Konk runs a series of commands serially or concurrently.

## Why?

There are two npm packages I frequently already use for running npm scripts
serially or concurrently: `npm-run-all` and `concurrently`. I built konk
because I wanted something that could run serially and concurrently and did not
need to be installed as an npm package (note, however, that konk can be
installed from npm). In addition, I wanted to be able to use the same command
line interface to run processes defined in a Procfile. Finally, I have always
been curious how to build such a command line interface, so this is also a
learning exercise for me.

There are currently feature gaps between `npm-run-all` and `concurrently`, but
I am working to fill them when I have time.

## Installation

Install via Homebrew:

```shell
brew install jclem/tap/konk
```

Or, use or install directly from npm:

```shell
npx konk
```

```shell
npm install -g konk
```

## Usage

### Run a Procfile

Given a procfile defined as follows:

```shell
a: echo A
b: echo B
c: echo C
```

Run the procfile commands concurrently:

```shell
konk procfile
```

```text
[a] a
[a] exit status: 0
[b] b
[b] exit status: 0
[c] c
[c] exit status: 0
```

### Run Commands

Commands can be provided as arguments to run them as-is, or they can be
provided with the `-n/--npm` flag to specify that they be read out of the
`scripts` section of the `package.json` file in the current working directory,
and then run.

#### Examples

Run three commands serially:

```shell
konk run s "echo A" "echo B" "echo C"
```

```text
[0] A
[0] exit status: 0
[1] B
[1] exit status: 0
[2] C
[2] exit status: 0
```

Run three commands serially with custom labels:

```shell
konk run s -l "1A" "echo A" -l "2B" "echo B" -l "3C" "echo C"
```

```text
[1A] A
[1A] exit status: 0
[2B] B
[2B] exit status: 0
[3C] C
[3C] exit status: 0
```

Run commands concurrently, using the commands themselves as labels:

```shell
konk run c -L "echo A; sleep 1; echo D" "echo B" "echo C"
```

```text
[echo A; sleep 1; echo D] A
[echo B                 ] B
[echo B                 ] exit status: 0
[echo C                 ] C
[echo C                 ] exit status: 0
[echo A; sleep 1; echo D] D
[echo A; sleep 1; echo D] exit status: 0
```

Run commands from `package.json`:

In this example, we specify a script "check" in the `package.json` file using
konk that runs all other scripts with the prefix "check:". We use `-c` to
specify that we should continue running other commands if one exits with a
non-zero exit code, `-g` specify that all of the output for a given command
should be aggregated together (rather than interleaved with other concurrent
commands), and `-L` to specify that the command itself should be used as the
label.

```json
{
    "scripts": {
        "check": "konk run c -cgL -n'check:*''",
        "check:lint": "eslint .",
        "check:format": "prettier --check .",
    }
}
```

```shell
npm run check
```

```text
[npm run check:format] 
[npm run check:format] > alight@0.0.0 check:format
[npm run check:format] > prettier --check .
[npm run check:format] 
[npm run check:format] Checking formatting...
[npm run check:format] All matched files use Prettier code style!
[npm run check:format] exit status: 0
[npm run check:lint  ] 
[npm run check:lint  ] > alight@0.0.0 check:lint
[npm run check:lint  ] > eslint .
[npm run check:lint  ] 
[npm run check:lint  ] exit status: 0
```

### CLI Help

#### Run Procfile

```text
Run commands defined in a Procfile (alias: p)

Usage: konk procfile [OPTIONS]

Options:
      --color <COLOR>                Enable color output [possible values: true, false]
  -c, --continue-on-failure          Continue running commands after any failures
      --env-file <ENV_FILE>          Path to a .env file to load [default: .env]
      --kill-timeout <KILL_TIMEOUT>  Time (in seconds) for commands to exit after receiving a SIGINT/SIGTERM before a SIGKILL is sent to them [default: 10]
      --no-env-file                  Do not load the .env file
      --no-environment               Do not inherit runtime environment variables
      --no-label                     Do not attach label to output
      --no-subshell                  Do not run commands with a subshell
      --show-pid                     Include command PID in output
  -h, --help                         Print help
```

#### Run Commands Serially

```text
Run commands serially (alias: s)

Usage: konk run serially [OPTIONS] [COMMANDS]...

Arguments:
  [COMMANDS]...  

Options:
  -b, --bun                          Run package.json scripts with Bun
      --color <COLOR>                Enable color output [possible values: true, false]
  -L, --command-as-label             Use each command as its own label
  -c, --continue-on-failure          Continue running commands after any failures
      --kill-timeout <KILL_TIMEOUT>  Time (in seconds) for commands to exit after receiving a SIGINT/SIGTERM before a SIGKILL is sent to them [default: 10]
  -l, --label <LABELS>               Label prefix for each command (must match given number of commands)
      --no-environment               Do not inherit runtime environment variables
      --no-label                     Do not attach label to output
      --no-subshell                  Do not run commands with a subshell
  -n, --npm <NPM>                    Run script defined in package.json by name
      --show-pid                     Include command PID in output
  -h, --help                         Print help
```

#### Run Commands Concurrently

```text
Run commands concurrently (alias: c)

Usage: konk run concurrently [OPTIONS] [COMMANDS]...

Arguments:
  [COMMANDS]...  

Options:
  -g, --aggregate-output             Aggregate command output
  -b, --bun                          Run package.json scripts with Bun
      --color <COLOR>                Enable color output [possible values: true, false]
  -L, --command-as-label             Use each command as its own label
  -c, --continue-on-failure          Continue running commands after any failures
      --kill-timeout <KILL_TIMEOUT>  Time (in seconds) for commands to exit after receiving a SIGINT/SIGTERM before a SIGKILL is sent to them [default: 10]
  -l, --label <LABELS>               Label prefix for each command (must match given number of commands)
      --no-environment               Do not inherit runtime environment variables
      --no-label                     Do not attach label to output
      --no-subshell                  Do not run commands with a subshell
  -n, --npm <NPM>                    Run script defined in package.json by name
      --show-pid                     Include command PID in output
  -h, --help                         Print help
```
