class ThreeVa < Formula
  desc "Secure-by-default JavaScript and TypeScript runtime. Deny-by-default permissions, no pm2 needed, post-install scripts blocked unconditionally."
  homepage "https://github.com/OdinoCano/3va"
  license "MIT"
  version "2.1.1"

  on_macos do
    on_intel do
      url "https://github.com/OdinoCano/3va/releases/download/v2.1.1/3va-v2.1.1-x86_64-apple-darwin.tar.gz"
      sha256 "c5890a9d45d23a55b920384c3bd7169a477c3326229acbbc01b33a695f89b227"
    end
    on_arm do
      url "https://github.com/OdinoCano/3va/releases/download/v2.1.1/3va-v2.1.1-aarch64-apple-darwin.tar.gz"
      sha256 "d977de6f21af3fa9d014190f047b4e5f7050b49fb8d2906a41618eb02d25f233"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/OdinoCano/3va/releases/download/v2.1.1/3va-v2.1.1-x86_64-unknown-linux-gnu.tar.gz"
      sha256 "9cf0d553a5881a6c64b8863b9df96ced8892a9e451b3c9932e03bb1f885e4373"
    end
    on_arm do
      url "https://github.com/OdinoCano/3va/releases/download/v2.1.1/3va-v2.1.1-aarch64-unknown-linux-gnu.tar.gz"
      sha256 "5d2db6768e39dfeb5301cd1b641355edb50753f4b64e0afd7c40aa530dc9a4b8"
    end
  end

  def install
    bin.install "3va"
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/3va --version")
  end
end
