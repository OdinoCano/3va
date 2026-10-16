class ThreeVa < Formula
  desc "Secure-by-default JavaScript and TypeScript runtime"
  homepage "https://github.com/OdinoCano/3va"
  url "https://github.com/OdinoCano/3va/archive/refs/tags/v2.13.0.tar.gz"
  sha256 "82c1ca95b254114df3123621cfc307b4b9369a13da0102ddb35a7aeb0e360fc4"
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
