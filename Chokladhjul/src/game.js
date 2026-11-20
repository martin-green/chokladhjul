const canvas = document.getElementById('canvas');
const bgcanvas = document.getElementById('background-layer');
const context = canvas.getContext('2d', { alpha: true, desynchronized: true });
const bgcontext = bgcanvas.getContext('2d', { alpha: false, desynchronized: true });

const fiftyFiftyMode = true; // stjärnvinst/förstapris
const yofs = 20;
const DEG_TO_RAD = Math.PI / 180;

const TEXT_POS = {
	number: { x: 1200, y: 150 + yofs },
	prize: { x: 1110, y: 220 + yofs }
};

let runCount = 0;
let playFanfar = false;
let isRendering = false;

canvas.oncontextmenu = function () {
	return false;
};

const sndFanfar = new Audio('fanfar.mp3');
sndFanfar.preload = 'auto';

const tickPool = [new Audio('tick.mp3'), new Audio('tick.mp3'), new Audio('tick.mp3')];
for (let i = 0; i < tickPool.length; i += 1) {
	tickPool[i].preload = 'auto';
}
let tickPoolIndex = 0;

function playTick() {
	const tick = tickPool[tickPoolIndex];
	tickPoolIndex = (tickPoolIndex + 1) % tickPool.length;
	tick.currentTime = 0;
	void tick.play();
}

function loadImage(src) {
	return new Promise(function (resolve, reject) {
		const img = new Image();
		img.onload = function () { resolve(img); };
		img.onerror = reject;
		img.src = src;
	});
}

function drawRotated(ctx, img, angleDegrees, centerX, centerY, size) {
	ctx.save();
	ctx.translate(centerX, centerY);
	ctx.rotate(angleDegrees * DEG_TO_RAD);
	ctx.drawImage(img, -size / 2, -size / 2, size, size);
	ctx.restore();
}

function drawTextWithShadow(ctx, text, x, y, font, color, shadowColor, shadowOffset) {
	ctx.font = font;
	ctx.fillStyle = shadowColor;
	ctx.fillText(text, x + shadowOffset.x, y + shadowOffset.y);
	ctx.fillStyle = color;
	ctx.fillText(text, x, y);
}

function drawBackground() {
	const gradient = bgcontext.createLinearGradient(0, 0, 0, bgcanvas.height);
	gradient.addColorStop(0, '#115577');
	gradient.addColorStop(1, '#007744');

	bgcontext.fillStyle = gradient;
	bgcontext.fillRect(0, 0, bgcanvas.width, bgcanvas.height);
}

function drawLogo(imgLogo) {
	bgcontext.drawImage(imgLogo, canvas.width - imgLogo.width - 32, canvas.height - imgLogo.height - 128);
}

class Wheel {
	constructor(img, x, y, numberCount, imgArrow) {
		this.x = x;
		this.y = y;
		this.angle = 0;
		this.angleSpeed = 1;
		this.angleSlowdown = 0.72; // degrees per second
		this.running = false;
		this.lastNum = 1;
		this.numbers = numberCount;
		this.img = img;
		this.imgArrow = imgArrow || null;
		this.size = Math.min(this.img.width, this.img.height);
		this.invSectorAngle = this.numbers / 360;
	}

	start() {
		this.angleSpeed = Math.max(this.angleSpeed, 8 * (0.7 + Math.random()));
		this.angleSlowdown = 0.72;
		this.running = true;
	}

	update(deltaSeconds) {
		if (!this.running) {
			return false;
		}

		this.angle += this.angleSpeed;
		if (this.angle >= 360) {
			this.angle -= 360;
		}

		this.angleSpeed -= this.angleSlowdown * deltaSeconds;
		if (this.angleSpeed <= 0) {
			this.angleSpeed = 0;
			this.running = false;
		}

		return true;
	}

	updateAudio() {
		const num = this.getNumber();
		if (this.lastNum !== num) {
			this.lastNum = num;
			playTick();
		}
	}

	getNumber() {
		return (this.numbers + Math.floor(0.5 - this.angle * this.invSectorAngle)) % this.numbers + 1;
	}

	getPriceType() {
		const num = this.getNumber();
		if (!fiftyFiftyMode) {
			if (num === 1) {
				return 0;
			}
			if (num === 2 || num === 8) {
				return 1;
			}
		} else {
			return num % 2 !== 0 ? 0 : 1;
		}
		return 2;
	}

	draw(ctx) {
		if (this.imgArrow === null) {
			drawRotated(ctx, this.img, this.angle, this.x, this.y, this.size);
			return;
		}

		const arrowSize = Math.min(this.imgArrow.width, this.imgArrow.height);
		drawRotated(ctx, this.imgArrow, this.angle, this.x, this.y, arrowSize);
	}
}

function drawMainWheelPointer(mainWheel) {
	const y = mainWheel.y - mainWheel.img.height / 2 - 10;

	bgcontext.fillStyle = 'black';
	bgcontext.beginPath();
	bgcontext.moveTo(mainWheel.x - 19, y - 6);
	bgcontext.lineTo(mainWheel.x, y + 34);
	bgcontext.lineTo(mainWheel.x + 19, y - 6);
	bgcontext.closePath();
	bgcontext.fill();

	bgcontext.fillStyle = 'orange';
	bgcontext.beginPath();
	bgcontext.moveTo(mainWheel.x - 10, y);
	bgcontext.lineTo(mainWheel.x, y + 27);
	bgcontext.lineTo(mainWheel.x + 10, y);
	bgcontext.closePath();
	bgcontext.fill();
}

let assets = null;
let mainWheel = null;
let prizeWheel = null;
let lastFrameTime = 0;

function drawBackgroundLayer() {
	drawBackground();
	drawLogo(assets.logo);

	// Prize wheel base is static; only arrow rotates on the foreground canvas.
	bgcontext.drawImage(
		assets.prize,
		prizeWheel.x - prizeWheel.size / 2,
		prizeWheel.y - prizeWheel.size / 2,
		prizeWheel.size,
		prizeWheel.size
	);

	drawMainWheelPointer(mainWheel);
}

function clearForeground() {
	context.clearRect(0, 0, canvas.width, canvas.height);
}

function drawResultLabels() {
	if (runCount <= 0) {
		return;
	}

	drawTextWithShadow(
		context,
		String(mainWheel.getNumber()),
		TEXT_POS.number.x,
		TEXT_POS.number.y,
		'80px Arial',
		'#fff',
		'#000',
		{ x: 3, y: 3 }
	);

	let prizeText = '';
	switch (prizeWheel.getPriceType()) {
		case 0:
			prizeText = 'Stjärnvinst';
			break;
		case 1:
			prizeText = 'Förstapris';
			break;
		default:
			prizeText = '';
			break;
	}

	if (prizeText.length > 0) {
		drawTextWithShadow(
			context,
			prizeText,
			TEXT_POS.prize.x,
			TEXT_POS.prize.y,
			'60px Arial',
			'#fff',
			'#000',
			{ x: 3, y: 3 }
		);
	}
}

function drawForeground() {
	clearForeground();
	mainWheel.draw(context);
	prizeWheel.draw(context);

	if (!mainWheel.running && !prizeWheel.running) {
		drawResultLabels();
	}
}

function requestRender() {
	if (isRendering) {
		return;
	}
	isRendering = true;
	lastFrameTime = performance.now();
	window.requestAnimationFrame(renderFrame);
}

function renderFrame(now) {
	const deltaSeconds = Math.min((now - lastFrameTime) / 1000, 0.05);
	lastFrameTime = now;

	const mainChanged = mainWheel.update(deltaSeconds);
	const prizeChanged = prizeWheel.update(deltaSeconds);
	const isSpinning = mainWheel.running || prizeWheel.running;

	if (mainChanged) {
		mainWheel.updateAudio();
	}

	drawForeground();

	if (isSpinning) {
		window.requestAnimationFrame(renderFrame);
		return;
	}

	isRendering = false;

	if (playFanfar && prizeWheel.getPriceType() === 0) {
		void sndFanfar.play();
		playFanfar = false;
	}
}

function resizeCanvas() {
	const width = window.innerWidth;
	const height = window.innerHeight;

	if (canvas.width === width && canvas.height === height) {
		return;
	}

	canvas.width = width;
	canvas.height = height;
	bgcanvas.width = width;
	bgcanvas.height = height;

	if (assets !== null && mainWheel !== null && prizeWheel !== null) {
		drawBackgroundLayer();
		drawForeground();
	}
}

function startWheel() {
	mainWheel.start();
	prizeWheel.start();
	runCount += 1;
	playFanfar = true;
	requestRender();
}

function initWheels() {
	mainWheel = new Wheel(assets.main, 600, assets.main.height / 2 + 50 + yofs, 24, null);
	prizeWheel = new Wheel(assets.prize, 1600, assets.main.height / 2 + 50 + yofs, 8, assets.arrow);
	mainWheel.lastNum = mainWheel.getNumber();
	prizeWheel.lastNum = prizeWheel.getNumber();
}

function init() {
	Promise.all([
		loadImage('tynnered-logo-small.png'),
		loadImage(fiftyFiftyMode ? 'prizewheel2.png' : 'prizewheel.png'),
		loadImage('arrow.png'),
		loadImage('wof.png')
	]).then(function (loaded) {
		assets = {
			logo: loaded[0],
			prize: loaded[1],
			arrow: loaded[2],
			main: loaded[3]
		};

		initWheels();
		resizeCanvas();
		drawBackgroundLayer();
		drawForeground();

		canvas.addEventListener('click', startWheel, false);
		window.addEventListener('resize', resizeCanvas, false);
	}).catch(function (err) {
		console.error('Failed to load assets', err);
	});
}

window.onload = init;

