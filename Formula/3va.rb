class ThreeVa < Formula
  desc "Secure-by-default JavaScript and TypeScript runtime. Deny-by-default permissions, no pm2 needed, post-install scripts blocked unconditionally."
  homepage "https://github.com/OdinoCano/3va"
  license "MIT"
  version "2.1.1"

  on_macos do
    on_intel do
      url "https://github.com/OdinoCano/3va/releases/download/2.1.1/3va-2.1.1-x86_64-apple-darwin.tar.gz"
      sha256 "dc6fbee7a6be71d76ed279c86d05b6a1dcd663707ff95c4eead46c5998367918"
    end
    on_arm do
      url "https://github.com/OdinoCano/3va/releases/download/2.1.1/3va-2.1.1-aarch64-apple-darwin.tar.gz"
      sha256 "cd8b142ecee12a6496a35fae02ac4d01e697a18a70c2a655ab987afe2415d6e1"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/OdinoCano/3va/releases/download/2.1.1/3va-2.1.1-x86_64-unknown-linux-gnu.tar.gz"
      sha256 "d775a9878c421bd61366a9865f5d205f32357ad6f49cc339a4e6c2f5aa4c8d0b"
    end
    on_arm do
      url "https://github.com/OdinoCano/3va/releases/download/2.1.1/3va-2.1.1-aarch64-unknown-linux-gnu.tar.gz"
      sha256 "7d88fdeceddb2cf892c4fcc21f99d2f7bce572805c28b78c0f353762e937cded"
    end
  end

  def install
    bin.install "3va"
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/3va --version")
  end
end
