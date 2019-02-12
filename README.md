# dep

A very basic, git based, flat dependency manager

Currently only public repos or repos with ssh are supported. So, no user-pass authentication.

## Commands

```c
dep global // prints the global config path
dep init   // creates an empty project config
dep update // updates all dependencies
```

## Sample Config

The configuration format is heavily inspired by the cargo package format, with some minor changes.

```toml
[project]
# required
name = 'dep'

# optional
# if lib-dir isn't set, default-lib-dir (defined in $HOME/.deprc) is used
lib-dir = 'VENDOR'

authors = ['hardliner66']
descrption = 'My cool project'
homepage = 'https://github.com/hardliner66/dep'
repository = 'https://github.com/hardliner66/dep'

git-server = 'git.myserver.com'

[dependencies]
# public git repo
some_repo = { git = 'https://my.gitserver.com/user/some_repo' }

# private git repo
some_private_repo = { git = 'git@my.gitserver.com:user/some_private_repo' }

# alternative syntax for private repos (only if git-server is set)
some_private_repo2 = { repo = 'user/some_private_repo2' }

# branches
some_other_private_repo = { git = 'git@my.gitserver.com:user/some_other_private_repo', branch = 'feature3' }

#local folders
some_local_repo = { path = '../some/local/folder' }
```

## TODOs / Planed features

- [ ] write better documentation
- [ ] local overrides file
