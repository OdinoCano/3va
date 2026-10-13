class ThreeVa < Formula
  desc "Secure-by-default JavaScript and TypeScript runtime"
  homepage "https://github.com/OdinoCano/3va"
  url "https://github.com/OdinoCano/3va/archive/refs/tags/v2.12.2.tar.gz"
  sha256 "5c75aa441c5a4cf7702ccae3ea83996d8d73e9f0381b99a6cf6282c430726c34"
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
