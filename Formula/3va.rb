class ThreeVa < Formula
  desc "Secure-by-default JavaScript and TypeScript runtime. Deny-by-default permissions, no pm2 needed, post-install scripts blocked unconditionally."
  homepage "https://github.com/OdinoCano/3va"
  license "MIT"
  version "2.12.0"

  on_macos do
    on_intel do
      url "https://github.com/OdinoCano/3va/releases/download/v2.12.0/3va-v2.12.0-x86_64-apple-darwin.tar.gz"
      sha256 "63e6d4281a6681c56b70438dd0a79f5fe4bc38c498f2eefe9933e62691172cbf"
    end
    on_arm do
      url "https://github.com/OdinoCano/3va/releases/download/v2.12.0/3va-v2.12.0-aarch64-apple-darwin.tar.gz"
      sha256 "7be8e7c156449d26957c1a144bffbfe94879b1f15b702308fefe86802282dcdd"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/OdinoCano/3va/releases/download/v2.12.0/3va-v2.12.0-x86_64-unknown-linux-gnu.tar.gz"
      sha256 "a95a1bea4c2cb4b631b050c6ac963d3505bafff6039d32a2a980cde6a4f61e2e"
    end
    on_arm do
      url "https://github.com/OdinoCano/3va/releases/download/v2.12.0/3va-v2.12.0-aarch64-unknown-linux-gnu.tar.gz"
      sha256 "c4cc223c99088d7b15c9a3df9232a70ee586bb128e6f92bcd8f714ffc83bad47"
    end
  end

  def install
    bin.install "3va"
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/3va --version")
  end
end
