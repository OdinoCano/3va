class ThreeVa < Formula
  desc "Secure-by-default JavaScript and TypeScript runtime. Deny-by-default permissions, no pm2 needed, post-install scripts blocked unconditionally."
  homepage "https://github.com/OdinoCano/3va"
  license "MIT"
  version "2.0.3"

  on_macos do
    on_intel do
      url "https://github.com/OdinoCano/3va/releases/download/v2.0.3/3va-v2.0.3-x86_64-apple-darwin.tar.gz"
      sha256 "c24cbddf72af2771f793acffe265e5ddf6d96802607e3d8673d4afc65e829867"
    end
    on_arm do
      url "https://github.com/OdinoCano/3va/releases/download/v2.0.3/3va-v2.0.3-aarch64-apple-darwin.tar.gz"
      sha256 "04fab025e98063b7919e78a549282358a2ba517ed129ff2aa9fb9b5f70670012"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/OdinoCano/3va/releases/download/v2.0.3/3va-v2.0.3-x86_64-unknown-linux-gnu.tar.gz"
      sha256 "ce5015b8147bbada4d38c18371b3d18e6e7d86e3ce8c423f5ef415d1cfda1679"
    end
    on_arm do
      url "https://github.com/OdinoCano/3va/releases/download/v2.0.3/3va-v2.0.3-aarch64-unknown-linux-gnu.tar.gz"
      sha256 "d6eb8a618adc5db565dea4923ff0743d118d01f55006abd43967f8d6726cc0d7"
    end
  end

  def install
    bin.install "3va"
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/3va --version")
  end
end
