class ThreeVa < Formula
  desc "Secure-by-default JavaScript and TypeScript runtime. Deny-by-default permissions, no pm2 needed, post-install scripts blocked unconditionally."
  homepage "https://github.com/OdinoCano/3va"
  license "MIT"
  version "2.0.2"

  on_macos do
    on_intel do
      url "https://github.com/OdinoCano/3va/releases/download/v2.0.2/3va-v2.0.2-x86_64-apple-darwin.tar.gz"
      sha256 "c155182d5d37a96c097a9958a2ced44028e4d86e77413634cb463233430f56aa"
    end
    on_arm do
      url "https://github.com/OdinoCano/3va/releases/download/v2.0.2/3va-v2.0.2-aarch64-apple-darwin.tar.gz"
      sha256 "3f74f9ea40e1f08e5eeacc29b7a8bec877a88cddcae17830a3a32e328e6d9bac"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/OdinoCano/3va/releases/download/v2.0.2/3va-v2.0.2-x86_64-unknown-linux-gnu.tar.gz"
      sha256 "f02da6c4425fe77d1caed88bebec1a889d04cb2cfb94c769ade60dc9da5d379b"
    end
    on_arm do
      url "https://github.com/OdinoCano/3va/releases/download/v2.0.2/3va-v2.0.2-aarch64-unknown-linux-gnu.tar.gz"
      sha256 "ba176d93c1536db6114b2c6fbf407af33379a09885325bb4e9881ccd537e8183"
    end
  end

  def install
    bin.install "3va"
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/3va --version")
  end
end
