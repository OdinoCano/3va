class ThreeVa < Formula
  desc "Secure-by-default JavaScript and TypeScript runtime"
  homepage "https://github.com/OdinoCano/3va"
  url "https://github.com/OdinoCano/3va/archive/refs/tags/v2.12.2.tar.gz"
  sha256 "eba215b09c5113669bad7a4794e24eb3ef9ac351a1f0f098fb32901ac9b13393"
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
