# Submitting 3va to homebrew-core

homebrew-core requires a project to meet minimum traction criteria before
acceptance.  Current thresholds (check https://docs.brew.sh/Acceptable-Formulae):

- ≥ 75 GitHub stars on the formula's upstream repository, OR
- Demonstrably significant user base in the Homebrew community

## Steps when ready

1. Fork https://github.com/Homebrew/homebrew-core
2. Copy `Formula/3va.rb` from the root of this repo into `Formula/3/3va.rb`
   (homebrew-core organises formulae under their first character).
3. Run `brew audit --new 3va` and fix any warnings.
4. Run `brew style 3va` — must pass RuboCop.
5. Open a PR titled exactly: `3va 1.0.0 (new formula)`.
6. The PR body must include:
   - Why the software is notable / what problem it solves
   - Link to the GitHub Releases page showing downloads
   - Confirmation that the formula was tested on macOS Intel and Apple Silicon
7. Do NOT use the tap URL in the PR; homebrew-core downloads source tarballs,
   not precompiled binaries.  You will need to adapt the formula to build from
   source using the `rust` language block:

     depends_on "rust" => :build

     def install
       system "cargo", "install", *std_cargo_args, "--bin", "3va"
     end
