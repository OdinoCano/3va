class ThreeVa < Formula
  desc "Secure-by-default JavaScript and TypeScript runtime. Deny-by-default permissions, no pm2 needed, post-install scripts blocked unconditionally."
  homepage "https://github.com/OdinoCano/3va"
  license "MIT"
  version "2.12.3"

  on_macos do
    on_intel do
      url "https://github.com/OdinoCano/3va/releases/download/v2.12.3/3va-v2.12.3-x86_64-apple-darwin.tar.gz"
      sha256 "3e1347fc6cd1ec69143df4743937fa39164d43fe70f5794255d03a83da706d26"
    end
    on_arm do
      url "https://github.com/OdinoCano/3va/releases/download/v2.12.3/3va-v2.12.3-aarch64-apple-darwin.tar.gz"
      sha256 "6aad48e4c99e46f26dc1cb6f63201720a73e55419f144d28be0fa3e0643d04b7"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/OdinoCano/3va/releases/download/v2.12.3/3va-v2.12.3-x86_64-unknown-linux-gnu.tar.gz"
      sha256 "1b4060d1e19511b44bb0cd60caf474a2c0470f35a01c4c1cc46ecb0aed70af1d"
    end
    on_arm do
      url "https://github.com/OdinoCano/3va/releases/download/v2.12.3/3va-v2.12.3-aarch64-unknown-linux-gnu.tar.gz"
      sha256 "25f47d36a4c3fb8bf37d8dd24fb6b5c21e7c974fe4139d75337f38784a7db2dc"
    end
  end

  def install
    bin.install "3va"
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/3va --version")
  end
end
