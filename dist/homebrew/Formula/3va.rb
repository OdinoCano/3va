class ThreeVa < Formula
  desc "Secure-by-default JavaScript and TypeScript runtime"
  homepage "https://github.com/OdinoCano/3va"
  url "https://github.com/OdinoCano/3va/archive/refs/tags/v2.12.4.tar.gz"
  sha256 "301c3c695d3e47ab5cbec8b4d81ec79e1f86b44e647ed4517a9bb515eeb00606"
  license "MIT"

  depends_on "pkgconf" => :build
  depends_on "rust" => :build

  uses_from_macos "zlib"

  def install
    system "cargo", "install", *std_cargo_args(path: "crates/cli")
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/3va --version")
  end
end
