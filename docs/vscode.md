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
lets `forformat` use declarations from the rest of the repository when resolving identifier case.
It also discovers the repository's `[tool.forformat]` settings in `pyproject.toml`, if present.

Use **Format Document** (`Shift+Alt+F`) to test the setup.

[fortran-extension]: https://marketplace.visualstudio.com/items?itemName=fortran-lang.linter-gfortran
