class ThreeVa < Formula
  desc "Secure-by-default JavaScript and TypeScript runtime. Deny-by-default permissions, no pm2 needed, post-install scripts blocked unconditionally."
  homepage "https://github.com/OdinoCano/3va"
  license "MIT"
  version "2.6.0"

  on_macos do
    on_intel do
      url "https://github.com/OdinoCano/3va/releases/download/v2.6.0/3va-v2.6.0-x86_64-apple-darwin.tar.gz"
      sha256 "01742f5f8f654e2866e04f2d66f7e69460c74fbe888ab2d839ccce42374adf11"
    end
    on_arm do
      url "https://github.com/OdinoCano/3va/releases/download/v2.6.0/3va-v2.6.0-aarch64-apple-darwin.tar.gz"
      sha256 "a6e04c2b27472c44a10f2726d1ff1f0d9c5f80c0895b764b8359b26ccaf28d21"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/OdinoCano/3va/releases/download/v2.6.0/3va-v2.6.0-x86_64-unknown-linux-gnu.tar.gz"
      sha256 "a662727e715dffa070ed83e05cbbf1771c97652b31e086df1a5403ec4613aea5"
    end
    on_arm do
      url "https://github.com/OdinoCano/3va/releases/download/v2.6.0/3va-v2.6.0-aarch64-unknown-linux-gnu.tar.gz"
      sha256 "76a5c3e736f975c2e4377092e622330724dc2d1d4d210b1263c79a629e3a286f"
    end
  end

  def install
    bin.install "3va"
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/3va --version")
  end
end
