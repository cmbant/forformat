# Using `forformat` with VS Code

Until the Fortran VS Code extension supports `forformat` directly, configure it through the
extension's `findent` formatter adapter.

## Setup

1. Install the [Fortran VS Code extension][fortran-extension] and install `forformat` in an
   environment visible to VS Code:

   ```sh
   python -m pip install forformat
   ```

2. Create an executable `.vscode/findent` file in the repository:

   ```python
   #!/usr/bin/env python3
   import shutil
   import subprocess
   import sys

   executable = shutil.which("forformat")
   if executable is None:
       raise SystemExit("forformat is required to format Fortran files")

   result = subprocess.run(
       [executable, "--stdin", *sys.argv[1:]],
       input=sys.stdin.buffer.read(),
       capture_output=True,
       check=False,
   )
   sys.stdout.buffer.write(result.stdout)
   sys.stderr.buffer.write(result.stderr)
   raise SystemExit(result.returncode)
   ```

   Then run:

   ```sh
   chmod +x .vscode/findent
   ```

3. Add these workspace settings to `.vscode/settings.json`:

   ```json
   {
       "fortran.formatting.formatter": "findent",
       "fortran.formatting.path": ".vscode",
       "fortran.formatting.findentArgs": [
           "--project-context=${workspaceFolder}"
       ],
       "[fortran]": {
           "editor.formatOnSave": true
       }
   }
   ```

The wrapper passes the editor buffer through `forformat` on stdin. The `--project-context` setting
only selects the Git checkout used for semantic project analysis, so `forformat` can use declarations
from the rest of the repository when resolving identifier case. Configuration discovery remains
based on the wrapper process working directory; use `--config=/path/to/.forformat.toml` in
`findentArgs` when that is not the repository directory.

A native editor/plugin integration should prefer `--stdin-filename=FILE` when it knows the file
represented by the unsaved buffer. That single identity also supplies configuration discovery,
default project discovery, filename-aware source-form detection, relative `INCLUDE` resolution, and
stale on-disk shadowing. The virtual filename is not limited to forformat's filesystem source
extension allow-list, so editor-only names such as `.fypp` are accepted; pass `-ifree` when such an
unrecognized suffix needs an explicit free-form override.

Use **Format Document** (`Shift+Alt+F`) to test the setup.

[fortran-extension]: https://marketplace.visualstudio.com/items?itemName=fortran-lang.linter-gfortran
