class ThreeVa < Formula
  desc "Secure-by-default JavaScript and TypeScript runtime. Deny-by-default permissions, no pm2 needed, post-install scripts blocked unconditionally."
  homepage "https://github.com/OdinoCano/3va"
  license "MIT"
  version "2.1.2"

  on_macos do
    on_intel do
      url "https://github.com/OdinoCano/3va/releases/download/v2.1.2/3va-v2.1.2-x86_64-apple-darwin.tar.gz"
      sha256 "41c1fae6ce7a4dcd51ab0bf35a4b8ad9d26d71d2a7b197c06b21e1ea872c399d"
    end
    on_arm do
      url "https://github.com/OdinoCano/3va/releases/download/v2.1.2/3va-v2.1.2-aarch64-apple-darwin.tar.gz"
      sha256 "9751709c06478e418cdc2b438c29d45c2d17b59d0a0f6d515627005f151cdbf6"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/OdinoCano/3va/releases/download/v2.1.2/3va-v2.1.2-x86_64-unknown-linux-gnu.tar.gz"
      sha256 "199219ca9aaaa20eff117c294e68ceb21147a2e4ae84e6bc36bcc8f111824ce3"
    end
    on_arm do
      url "https://github.com/OdinoCano/3va/releases/download/v2.1.2/3va-v2.1.2-aarch64-unknown-linux-gnu.tar.gz"
      sha256 "6a7136cd8bd2e3e4530c8c4f4d69e074ba9e499f8adc9391024b32635f6ad850"
    end
  end

  def install
    bin.install "3va"
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/3va --version")
  end
end
