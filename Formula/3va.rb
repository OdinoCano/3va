class ThreeVa < Formula
  desc "Secure-by-default JavaScript and TypeScript runtime. Deny-by-default permissions, no pm2 needed, post-install scripts blocked unconditionally."
  homepage "https://github.com/OdinoCano/3va"
  license "MIT"
  version "2.1.0"

  on_macos do
    on_intel do
      url "https://github.com/OdinoCano/3va/releases/download/v2.1.0/3va-v2.1.0-x86_64-apple-darwin.tar.gz"
      sha256 "0eed7a5035dac313565d2ed699c0563173be3c25a55c0d9baf94be491e6ce152"
    end
    on_arm do
      url "https://github.com/OdinoCano/3va/releases/download/v2.1.0/3va-v2.1.0-aarch64-apple-darwin.tar.gz"
      sha256 "3768e614dafb3e4409e15812f9ae1492cb29a31786f649a981de987441d56125"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/OdinoCano/3va/releases/download/v2.1.0/3va-v2.1.0-x86_64-unknown-linux-gnu.tar.gz"
      sha256 "f4568dd55cead2b40f2a5c4e383efd0dc8c1b2076a31a45b032d8294f1a95e9f"
    end
    on_arm do
      url "https://github.com/OdinoCano/3va/releases/download/v2.1.0/3va-v2.1.0-aarch64-unknown-linux-gnu.tar.gz"
      sha256 "ccc871d5a342981ab145c65afb0be7e66dcc18d40f247f5a8cd0ee2c8ba59395"
    end
  end

  def install
    bin.install "3va"
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/3va --version")
  end
end
