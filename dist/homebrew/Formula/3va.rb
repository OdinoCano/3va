class ThreeVa < Formula
  desc "Secure-by-default JavaScript and TypeScript runtime"
  homepage "https://github.com/OdinoCano/3va"
  url "https://github.com/OdinoCano/3va/archive/refs/tags/v2.1.2.tar.gz"
  sha256 "f70ba3a621bfe1c069a5c4e6977ad8b28afeef2090208f90cfaec86c321594ec"
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
