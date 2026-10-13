class ThreeVa < Formula
  desc "Secure-by-default JavaScript and TypeScript runtime. Deny-by-default permissions, no pm2 needed, post-install scripts blocked unconditionally."
  homepage "https://github.com/OdinoCano/3va"
  license "MIT"
  version "2.12.2"

  on_macos do
    on_intel do
      url "https://github.com/OdinoCano/3va/releases/download/v2.12.2/3va-v2.12.2-x86_64-apple-darwin.tar.gz"
      sha256 "b1a102abf097c97f6da1160b579b8c7ff0b995e90c5f04bf2fe33654dc527988"
    end
    on_arm do
      url "https://github.com/OdinoCano/3va/releases/download/v2.12.2/3va-v2.12.2-aarch64-apple-darwin.tar.gz"
      sha256 "d99e78e4272cdce94dec8752b1cad5410ccf5371d968bbb64d7b622c99940de7"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/OdinoCano/3va/releases/download/v2.12.2/3va-v2.12.2-x86_64-unknown-linux-gnu.tar.gz"
      sha256 "71748b90ac5cd835399327d1323e9eafb612079ebad50cb554c4c7b6836b7d82"
    end
    on_arm do
      url "https://github.com/OdinoCano/3va/releases/download/v2.12.2/3va-v2.12.2-aarch64-unknown-linux-gnu.tar.gz"
      sha256 "a4b16277ac6bfbe82077934a61f05cb36783e7798057506ed7cac108964fff64"
    end
  end

  def install
    bin.install "3va"
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/3va --version")
  end
end
