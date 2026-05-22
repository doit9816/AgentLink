# Bundle desktop AgentLink CLI

## Why

The packaged macOS desktop app currently launches without a usable AgentLink bridge binary on disk for the default connection. Users can open the `.app`, but the desktop client still reports that the AgentLink executable does not exist unless they also build the repository or manually locate a separate CLI binary.

That makes the installed app feel broken even though the desktop shell itself is present.

## What Changes

- stage the root `agentlink` CLI into the desktop bundle resources during packaged builds
- teach desktop path discovery to look inside packaged app resources for the bundled CLI
- default packaged desktop connections to the bundled CLI name instead of leaving the executable path blank

## Impact

- packaged macOS desktop builds become usable without a separate manual CLI install
- release packaging now compiles the root `agentlink` binary before bundling the desktop app
