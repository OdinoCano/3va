class ThreeVa < Formula
  desc "Secure-by-default JavaScript and TypeScript runtime. Deny-by-default permissions, no pm2 needed, post-install scripts blocked unconditionally."
  homepage "https://github.com/OdinoCano/3va"
  license "MIT"
  version "1.0.0"

  on_macos do
    on_intel do
      url "https://github.com/OdinoCano/3va/releases/download/v1.0.0/3va-v1.0.0-x86_64-apple-darwin.tar.gz"
      sha256 "SHA256_DARWIN_X64"
    end

    on_arm do
      url "https://github.com/OdinoCano/3va/releases/download/v1.0.0/3va-v1.0.0-aarch64-apple-darwin.tar.gz"
      sha256 "SHA256_DARWIN_ARM64"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/OdinoCano/3va/releases/download/v1.0.0/3va-v1.0.0-x86_64-unknown-linux-gnu.tar.gz"
      sha256 "SHA256_LINUX_X64"
    end

    on_arm do
      url "https://github.com/OdinoCano/3va/releases/download/v1.0.0/3va-v1.0.0-aarch64-unknown-linux-gnu.tar.gz"
      sha256 "SHA256_LINUX_ARM64"
    end
  end

  def install
    bin.install "3va"
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/3va --version")
  end
end
