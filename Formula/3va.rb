class ThreeVa < Formula
  desc "Secure-by-default JavaScript and TypeScript runtime. Deny-by-default permissions, no pm2 needed, post-install scripts blocked unconditionally."
  homepage "https://github.com/OdinoCano/3va"
  license "MIT"
  version "2.0.0"

  on_macos do
    on_intel do
      url "https://github.com/OdinoCano/3va/releases/download/v2.0.0/3va-v2.0.0-x86_64-apple-darwin.tar.gz"
      sha256 "3f8b31d47875cf9a245c464fc6fa46c081eb924de22fb04d13c8c84042530dc3"
    end
    on_arm do
      url "https://github.com/OdinoCano/3va/releases/download/v2.0.0/3va-v2.0.0-aarch64-apple-darwin.tar.gz"
      sha256 "a48456410754037f6022559ce8298f861940b3e1cb00b0184ab2e13133763a68"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/OdinoCano/3va/releases/download/v2.0.0/3va-v2.0.0-x86_64-unknown-linux-gnu.tar.gz"
      sha256 "b3ae6c9f79933e6a44e55feba401cf5f1ece084eb646af76427cd4efb1caab6a"
    end
    on_arm do
      url "https://github.com/OdinoCano/3va/releases/download/v2.0.0/3va-v2.0.0-aarch64-unknown-linux-gnu.tar.gz"
      sha256 "32da58329d61f58f95e6d69e71be767f2cb3f519fc9163b6233e702a0b06204f"
    end
  end

  def install
    bin.install "3va"
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/3va --version")
  end
end
