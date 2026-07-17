class ThreeVa < Formula
  desc "Secure-by-default JavaScript and TypeScript runtime. Deny-by-default permissions, no pm2 needed, post-install scripts blocked unconditionally."
  homepage "https://github.com/OdinoCano/3va"
  license "MIT"
  version "2.4.0"

  on_macos do
    on_intel do
      url "https://github.com/OdinoCano/3va/releases/download/v2.4.0/3va-v2.4.0-x86_64-apple-darwin.tar.gz"
      sha256 "1cbd8c7ac15212ec0823feed8efbe9d69d0fd0ccab5968373ce88365fe07d68b"
    end
    on_arm do
      url "https://github.com/OdinoCano/3va/releases/download/v2.4.0/3va-v2.4.0-aarch64-apple-darwin.tar.gz"
      sha256 "9bb997a5428bfd2a6655d7018776dc505af9bed290c99b937bdc336531411594"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/OdinoCano/3va/releases/download/v2.4.0/3va-v2.4.0-x86_64-unknown-linux-gnu.tar.gz"
      sha256 "40ec5c8f5be775600ad5a593d9af569eb8d0a2a84c79a45f59fddc72d57bfdbb"
    end
    on_arm do
      url "https://github.com/OdinoCano/3va/releases/download/v2.4.0/3va-v2.4.0-aarch64-unknown-linux-gnu.tar.gz"
      sha256 "1b5564d06adf939e77cfa68f9754f0bca0beec3f4463d85e3fb0e7e10d63c5b5"
    end
  end

  def install
    bin.install "3va"
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/3va --version")
  end
end
