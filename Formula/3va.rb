class ThreeVa < Formula
  desc "Secure-by-default JavaScript and TypeScript runtime. Deny-by-default permissions, no pm2 needed, post-install scripts blocked unconditionally."
  homepage "https://github.com/OdinoCano/3va"
  license "MIT"
  version "2.13.0"

  on_macos do
    on_intel do
      url "https://github.com/OdinoCano/3va/releases/download/v2.13.0/3va-v2.13.0-x86_64-apple-darwin.tar.gz"
      sha256 "805bff97611cc5343c355d14e2c024e8f970d458ab7bee70a3d66bd05a65856e"
    end
    on_arm do
      url "https://github.com/OdinoCano/3va/releases/download/v2.13.0/3va-v2.13.0-aarch64-apple-darwin.tar.gz"
      sha256 "29625d3259c44e6d38b336ad0c6f7578a55baf055a07508a522a9ef4eb1d9b0d"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/OdinoCano/3va/releases/download/v2.13.0/3va-v2.13.0-x86_64-unknown-linux-gnu.tar.gz"
      sha256 "42fa6fc46fedda32266d888fc6c19a5ee9490688895f8d623613f28ce6c36db2"
    end
    on_arm do
      url "https://github.com/OdinoCano/3va/releases/download/v2.13.0/3va-v2.13.0-aarch64-unknown-linux-gnu.tar.gz"
      sha256 "f7a5f3550008cb7b142b0a2d1af622c780810045f1ffec64c9a2bd91a976e8a9"
    end
  end

  def install
    bin.install "3va"
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/3va --version")
  end
end
