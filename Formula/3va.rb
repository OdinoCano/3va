class ThreeVa < Formula
  desc "Secure-by-default JavaScript and TypeScript runtime. Deny-by-default permissions, no pm2 needed, post-install scripts blocked unconditionally."
  homepage "https://github.com/OdinoCano/3va"
  license "MIT"
  version "2.12.5"

  on_macos do
    on_intel do
      url "https://github.com/OdinoCano/3va/releases/download/v2.12.5/3va-v2.12.5-x86_64-apple-darwin.tar.gz"
      sha256 "8a330c026b1b12a5ea3836561e6ce43cf68b856d5653b67f6fa34011212854a1"
    end
    on_arm do
      url "https://github.com/OdinoCano/3va/releases/download/v2.12.5/3va-v2.12.5-aarch64-apple-darwin.tar.gz"
      sha256 "4d78fb2962db9263a15967652cf15bd0f0efea58c365fc8b54a0c3c8fbc6dc11"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/OdinoCano/3va/releases/download/v2.12.5/3va-v2.12.5-x86_64-unknown-linux-gnu.tar.gz"
      sha256 "6ddf8c3aae33a2eca159d0b9c5ad49d5f04b58e158069b92a8c958545b906ebc"
    end
    on_arm do
      url "https://github.com/OdinoCano/3va/releases/download/v2.12.5/3va-v2.12.5-aarch64-unknown-linux-gnu.tar.gz"
      sha256 "67285b2044199ac6a8e1a09bc4dbfbc4942a5640ea89e5a7622fb044a206ef48"
    end
  end

  def install
    bin.install "3va"
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/3va --version")
  end
end
