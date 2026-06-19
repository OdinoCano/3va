class ThreeVa < Formula
  desc "Secure-by-default JavaScript and TypeScript runtime. Deny-by-default permissions, no pm2 needed, post-install scripts blocked unconditionally."
  homepage "https://github.com/OdinoCano/3va"
  license "MIT"
  version "2.0.4"

  on_macos do
    on_intel do
      url "https://github.com/OdinoCano/3va/releases/download/v2.0.4/3va-v2.0.4-x86_64-apple-darwin.tar.gz"
      sha256 "7fb344c533e099485ff39395e19b8a16ce1c8d1a70f89b38952e4b30d6aa5c6a"
    end
    on_arm do
      url "https://github.com/OdinoCano/3va/releases/download/v2.0.4/3va-v2.0.4-aarch64-apple-darwin.tar.gz"
      sha256 "75c971c0697986adaf484b783eac6a5d337daec87c5cfac73830e2f05ef96a28"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/OdinoCano/3va/releases/download/v2.0.4/3va-v2.0.4-x86_64-unknown-linux-gnu.tar.gz"
      sha256 "5e9e02743d81da6a00b22eea396d1610a33d1bc27372b5aa972d3834994e708b"
    end
    on_arm do
      url "https://github.com/OdinoCano/3va/releases/download/v2.0.4/3va-v2.0.4-aarch64-unknown-linux-gnu.tar.gz"
      sha256 "5a31a8f0737177a33ccda4df5408fc539af4cb99687f6733996b3d5be75a0df8"
    end
  end

  def install
    bin.install "3va"
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/3va --version")
  end
end
